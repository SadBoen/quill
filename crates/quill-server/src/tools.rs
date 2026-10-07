//! 对话可用的内置工具。
//!
//! **为什么先有这层**：`quill-provider` 的工具调用是完整的（`ToolSpec` 带 JSON
//! Schema、`ChatRequest::with_tools` 会把 `tools` 写进请求体、响应侧能从
//! `delta.tool_calls` 拼装），缺的是「有哪些工具」和「调完怎么办」。这一层补的就是
//! 那两件事，provider 一行没改。
//!
//! **执行结果必须回灌**。模型发出 `tool_calls` 之后，本轮回复里**没有正文**。
//! 若就此返回，界面会显示一条空消息，而用户的问题其实还没被回答。
//! 正确形状是：把工具结果作为 `role=tool` 的消息追加进上下文，再问一次模型，
//! 拿到它基于工具结果的真正回答。这与 OpenAI 的 tool-calling 往返一致。
//!
//! **安全边界**：工具只做只读的事。写文件、跑命令这类能改系统状态的**故意不做** ——
//! 一个能被网页里的任意文本驱动的执行器是远程代码执行，不是 Agent 能力。
//! 写入类操作等 MCP 接入后由用户显式配置允许的工具提供。

use std::sync::Arc;

use serde_json::{json, Value};

use quill_provider::{ToolCall, ToolSpec};

/// 一次对话里最多允许多少轮工具往返。
///
/// 每一轮都是一次真实的模型调用（本地模型 1~2 秒），不设上限的话一个
/// 「不断调用工具直到满意」的循环能把请求挂到超时，且每一轮都在烧 CPU。
/// 取 4 是够用的上限：真实任务（查两次专家、再查一次会话）用不到一半。
pub const MAX_TOOL_ROUNDS: usize = 4;

// 上限本身是个不变量：改成 0 会让工具永不执行，改得太大则每轮都在烧真实模型调用。
// 用**编译期断言**而不是单测 —— 这条断言在构造上就不会失败（所以 clippy 报
// assertions_on_constants 是对的），它的价值在于构建时就把改坏的人拦下。
const _: () = assert!(
    MAX_TOOL_ROUNDS > 0,
    "工具往返上限不能是 0，否则工具永远不执行"
);
const _: () = assert!(
    MAX_TOOL_ROUNDS <= 8,
    "工具往返上限过大：每轮都是一次真实模型调用"
);

/// 轮次用尽后、去掉工具再问一次时塞给模型的话。
///
/// 措辞要点：**让它把手上已有的信息说出来，而不是要求它「给出正确答案」**。
/// 工具查不到东西是正常结果（服务器没配、文件不存在、桩只回一句话），
/// 模型如实说明缺什么才是有用输出；逼它「想办法」只会让它继续要工具。
pub const FINAL_ANSWER_PROMPT: &str = "不要再调用任何工具了。现在请直接用你手上已有的信息回答：\
     说清楚你已经查到了什么、哪些信息拿不到、拿不到的原因是什么。\
     **不知道的项目直接写「不知道」。不要给估计值、不要给「大约」、\
     不要凭印象填一个看起来合理的数字。**\
     你现在没有任何工具可以核实，所以你说的每一个具体数字都可能是编的。";

/// 收尾调用在工具轨迹里用的标记名。界面上会看到这一条，
/// 它表示「这一段是去掉工具之后逼出来的正文」，不是用户调的工具。
pub const FINAL_ANSWER_MARKER: &str = "final_answer";

/// 单个工具结果回灌给模型时的长度上限。
///
/// 工具可能返回整份文件或长列表，全量回灌会让下一轮的上下文迅速膨胀。
/// 截断时**明说被截断了**，否则模型会以为自己看到的是全部。
const MAX_RESULT_CHARS: usize = 4_000;

/// 工具执行体。拿到参数，返回给模型看的文本。
///
/// 刻意用 `Result<String, String>` 而不是 `Result<Value, _>`：工具结果是给
/// **模型**读的文本，不是给前端看的结构化数据。`Err` 的内容会**原样**回灌给
/// 模型（让模型能自己纠正），也会记进服务端日志。
pub type ToolHandler = Arc<dyn Fn(&Value) -> Result<String, String> + Send + Sync>;

pub struct ToolRegistry {
    specs: Vec<ToolSpec>,
    handlers: Vec<(String, ToolHandler)>,
}

impl ToolRegistry {
    /// 当前实例可用的工具。没有工具时返回空 Vec —— 此时 provider 不会写
    /// `tools` 字段，请求体与接入前逐字节一致。
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn handler(&self, name: &str) -> Option<&ToolHandler> {
        self.handlers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h)
    }

    /// 注册一个工具。MCP 接入后，MCP 服务器 `tools/list` 返回的条目也走这里，
    /// 所以「内置」与「MCP 来的」在模型看来没有区别。
    pub fn register(&mut self, spec: ToolSpec, handler: ToolHandler) {
        let name = spec.name.clone();
        self.specs.retain(|s| s.name != name);
        self.specs.push(spec);
        self.handlers.retain(|(n, _)| n != &name);
        self.handlers.push((name, handler));
    }

    /// 执行一次工具调用。
    ///
    /// **未知工具必须返回 Err 而不是跳过**：模型如果幻觉出一个不存在的工具，
    /// 我们应当把这个事实告诉它（`Err` 的文本会回灌），而不是假装调用成功。
    /// 一个被静默吞掉的未知工具名，会让模型反复尝试同一个不存在的工具。
    pub fn call(&self, call: &ToolCall) -> Result<String, String> {
        let Some(handler) = self.handler(&call.name) else {
            let available: Vec<&str> = self.specs.iter().map(|s| s.name.as_str()).collect();
            return Err(format!(
                "没有名为「{}」的工具。当前可用：{}",
                call.name,
                if available.is_empty() {
                    "（无）".to_string()
                } else {
                    available.join("、")
                }
            ));
        };
        handler(&call.arguments)
    }

    /// 把工具结果整理成回灌给模型的文本，并做长度截断。
    pub fn render_result(&self, _call: &ToolCall, result: Result<String, String>) -> String {
        let body = match result {
            Ok(text) if text.trim().is_empty() => "（工具执行成功，但没有返回内容）".to_string(),
            Ok(text) => text,
            Err(detail) => format!("工具执行失败：{detail}"),
        };
        if body.chars().count() <= MAX_RESULT_CHARS {
            return body;
        }
        let kept: String = body.chars().take(MAX_RESULT_CHARS).collect();
        format!(
            "{kept}\n\n（结果过长，已截断到 {MAX_RESULT_CHARS} 字符；\
             如果需要后半部分，请缩小查询范围后重试。）"
        )
    }

    /// 本实例的内置工具。
    ///
    /// **按用户过滤**：`uid` 会一并传进来，工具查询走 `list_visible(&uid)`，
    /// 也就是和 `GET /api/experts` 同一套可见性规则。用一个「全体专家」
    /// 的工具会泄露别人的私有专家 —— 工具是模型触发的，等于用户自己点的。
    ///
    /// `app` 传 `Arc` 而不是 `&`，是为了让闭包能持有它；请求结束时
    /// registry 一起被丢弃，里面的 `Arc` 也随之释放。
    pub fn builtin(app: Arc<crate::state::AppState>, uid: quill_adapters::UserId) -> Self {
        Self::builtin_with_expert_tools(app, uid, true)
    }

    /// `include_expert_tools=false` 时**不挂** `list_experts` / `get_expert_detail`。
    ///
    /// **为什么需要这个开关**：会话的角色是用户在界面上选的，不是模型选的。
    /// 角色一旦定了，把这两个工具挂给模型就是纯噪音 —— 实测 11 条真实任务里有 5 条
    /// 以 `tool_loop_exhausted` 收场，而日志显示模型把 **4 轮预算里的 3 轮**
    /// 花在 `list_experts` → `get_expert_detail` 上反复试水，
    /// 真正该调的 `notes__*` 根本没轮到。见 ISSUE-041。
    pub fn builtin_with_expert_tools(
        app: Arc<crate::state::AppState>,
        uid: quill_adapters::UserId,
        include_expert_tools: bool,
    ) -> Self {
        let mut r = ToolRegistry {
            specs: Vec::new(),
            handlers: Vec::new(),
        };

        // 两个闭包各自持有一份 Arc：AppState 里的 DbBridge/LLM 都是 Arc，
        // 克隆是廉价的引用计数递增，不会复制任何运行状态。
        let app2 = Arc::clone(&app);

        // **一个专家都没有时，这两个工具一律不挂**；角色已经定了的时候也不挂。
        //
        // 前半条是实测（2026-10-06，跑 50 条 SkillsBench）：这台库上专家数是 **0**，
        // 于是 `list_experts` 每一条请求都挂着，而它**永远只能返回
        // 「没有匹配的专家。」**；`get_expert_detail` 也必然失败。
        // 4B 模型在这两个工具上反复重试（轨迹里连着三四个 `list_experts`），
        // 直到 `MAX_TOOL_ROUNDS` 用尽 —— 于是一条本来能答的任务被判成
        // 「连续 4 轮只调工具不给正文」。
        //
        // 给一个永远产不出东西的用户挂上这两个工具，是**纯粹的噪音**：
        // 既占工具表的 token，又把模型往一个必然空手的分支上引。
        //
        // **查库失败时不因此少挂** —— 那是 quill 自己的存储出问题，
        // 静默少挂两个工具会变成「模型好像没学过专家」且毫无迹象。
        let has_experts = match crate::api_experts::list_for_tools(&app, uid) {
            Ok(list) => !list.is_empty(),
            Err(e) => {
                eprintln!("[tools] 判断是否挂载专家工具时读库失败，按「有专家」挂上：{e}");
                true
            }
        };

        if has_experts && include_expert_tools {
            r.register(
                ToolSpec::new(
                    "list_experts",
                    "列出当前可用的专家（角色）。当你不知道有哪些专家可选时调用。",
                )
                .with_parameters(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "可选。按名称或描述做关键词过滤，省略则返回全部。"
                        }
                    },
                    "required": []
                })),
                Arc::new(move |args: &Value| {
                    let query = args
                        .get("query")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_lowercase();
                    let list = crate::api_experts::list_for_tools(&app, uid)?;
                    let mut names = Vec::new();
                    for e in list {
                        let name = e.display_name().to_lowercase();
                        let desc = e.description().to_lowercase();
                        if query.is_empty() || name.contains(&query) || desc.contains(&query) {
                            names.push(format!("{}（{}）", e.display_name(), e.id()));
                        }
                    }
                    if names.is_empty() {
                        return Ok("没有匹配的专家。".to_string());
                    }
                    Ok(names.join("、"))
                }),
            );

            r.register(
                ToolSpec::new(
                    "get_expert_detail",
                    "按名称查看某个专家的专长介绍。选专家之前想了解它擅长什么时调用。",
                )
                .with_parameters(json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "专家名称。list_experts 的返回值是「显示名（id）」，原样传回来即可；只传显示名或只传 id 也认。"
                        }
                    },
                    "required": ["name"]
                })),
                Arc::new(move |args: &Value| {
                    let name = args
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("缺少参数 name。")?
                        .trim()
                        .to_string();
                    if name.is_empty() {
                        return Err("name 不能是空串。".to_string());
                    }
                    let list = crate::api_experts::list_for_tools(&app2, uid)?;
                    let found = list
                        .into_iter()
                        .find(|e| {
                            matches_expert_query(e.id().as_str(), e.display_name(), &name)
                        })
                        .ok_or(format!(
                            "没有名为「{name}」的专家。\
                             下一步：list_experts 的返回值是「显示名（id）」这种形状，\
                             原样传回来就能命中；也接受只传显示名或只传 id。"
                        ))?;
                    Ok(format!(
                        "{}：{}",
                        found.display_name(),
                        if found.description().trim().is_empty() {
                            "（没有写简介）"
                        } else {
                            found.description()
                        }
                    ))
                }),
            );
        } // if has_experts —— 一个专家都没有时，上面这两个工具根本不进工具表

        r
    }

    /// 把该用户**已启用**的 SKILL 挂进工具表 —— 「SKILL 即工具」的落地点。
    ///
    /// 这一步是 async 而 `builtin` 是 sync：SKILL 的行在库里、正文在磁盘上，
    /// 两处都得读。`builtin` 只闭包 `AppState`、不发 IO，所以那个签名不动。
    ///
    /// **失败就整条请求失败，不静默降级成「只有内置工具」。** 静默降级是最难查
    /// 的一种故障：用户看到的是「模型好像没学过我的技能」，却没有任何迹象说明
    /// 是加载失败，而且每一轮都这样。宁可 500，也不要一个看起来正常的对话。
    ///
    /// 两种「行在、正文不在」的情况**跳过并记日志**，不注册工具：挂一个
    /// description 为空的工具进去，模型会调到一个必然没有产出的东西，而界面上
    /// 还显示这个技能是启用的。
    pub async fn with_skills(
        mut self,
        db: &crate::db::DbBridge,
        uid: quill_adapters::UserId,
        root: &std::path::Path,
    ) -> Result<Self, String> {
        let rows = crate::skills_repo::list(db, uid)
            .await
            .map_err(|e| format!("加载 SKILL 列表失败：{e}"))?;

        for row in rows {
            // 正文路径由 `api_extensions::skill_body_path` 一处算出来 ——
            // 这里自己拼一份，就会出现「界面显示已启用、模型读不到正文」。
            // 算不出路径（DB 里名字不合法）就退化成「磁盘上没正文」，
            // 与「文件被删了」同一种形状：下面的 `NotMounted` 会照常
            // 报出「库里有行但磁盘上没有正文」，不另造一套说法。
            let body = match crate::api_extensions::skill_body_path(root, &row.name) {
                Ok(p) => crate::api_extensions::read_skill_body(&p),
                Err(_) => String::new(),
            };
            match skill_visibility(&self.specs, &row, &body) {
                SkillVisibility::Visible => {}
                // 停用是用户的明确选择，不是故障 —— 记日志只会把真正的告警淹掉。
                SkillVisibility::Disabled => continue,
                SkillVisibility::NotMounted(why) => {
                    eprintln!("[tools] 跳过 SKILL {}：{why}", row.name);
                    continue;
                }
            }
            let spec = crate::skills_repo::as_tool_spec(&row, &body);
            let handler = skill_handler(&row.name, &body);
            self.register(spec, handler);
        }
        Ok(self)
    }

    /// 把该用户 MCP 服务器 `tools/list` 报出来的条目挂进工具表。
    ///
    /// 与 `with_skills` 的差别，也是这一层最要紧的一条取舍：
    /// **一台服务器连不上，不让整条对话失败。** `with_skills` 查库失败就 500，
    /// 因为那是 quill 自己的存储坏了，静默降级会变成「模型好像没学过我的技能」
    /// 且毫无迹象。而 MCP 服务器是**用户自己在外面起的进程**，它没起、装依赖失败、
    /// 改配置到一半 —— 这些都是正常状态，不该让聊天整体不可用。
    /// 所以：查库失败仍然整条请求失败（那是我们的问题），单台服务器失败只记日志跳过。
    ///
    /// **每次对话都真的握手一次。** 不缓存工具列表：缓存一旦过期，
    /// 界面上显示的工具数就会与模型实际拿到的对不上，而那种不一致没法从界面上看出来。
    /// 代价是每条消息都要把用户配的进程拉起来一遍 —— 这笔账记在 STATUS.md 的
    /// 「已知代价」里，真到扛不住时再上带 TTL 的缓存，并且**界面上要显示缓存年龄**。
    pub async fn with_mcp_tools(
        mut self,
        db: &crate::db::DbBridge,
        uid: quill_adapters::UserId,
        user_key: &str,
    ) -> Result<Self, String> {
        let rows = crate::mcp_repo::list(db, uid)
            .await
            .map_err(|e| format!("加载 MCP 服务器列表失败：{e}"))?;
        if rows.is_empty() {
            return Ok(self);
        }
        // **停用就是停用**：用户在配置里把 `enabled` 关掉的服务器，这一轮
        // 不能握手、也不能挂工具。
        //
        // 原来这里一行都没判，`enabled` 存进了库、从 API 读得出来，却对
        // 「模型这一轮真能调到什么」毫无影响 —— 与 ISSUE-008
        // （`tool_allowlist` 只存不用）是同一种病。表现是：用户明明在界面上
        // 关掉了一台服务器，它的工具照样出现在模型的工具表里，用户无从察觉。
        //
        // 与 `with_skills` 对停用技能的处理保持一致：跳过，不记日志 ——
        // 停用是用户的明确选择，不是故障，记日志只会把真正的告警淹掉。
        let rows = enabled_servers(rows);
        if rows.is_empty() {
            return Ok(self);
        }
        // 与 `api_extensions::mcp_body` 走的是**同一个** `discover_all`，
        // 所以「界面说挂了几个」与「模型这一轮真能调几个」出自同一次握手。
        let found = crate::mcp_client::discover_all(rows.clone()).await;
        for (row, d) in rows.iter().zip(found.iter()) {
            if !d.probe.connected {
                // 「没连上」与「连上了但没工具」是两回事，不合并成一条日志。
                eprintln!(
                    "[tools] MCP 服务器 {} 没挂上工具：{}",
                    row.name,
                    d.probe.error.as_deref().unwrap_or("（无原因）")
                );
                continue;
            }
            for tool in &d.tools {
                match mcp_tool_visibility(&self.specs, &row.name, tool) {
                    McpToolVisibility::Visible => {}
                    McpToolVisibility::NotMounted(why) => {
                        eprintln!(
                            "[tools] 跳过 MCP 工具 {}/{}：{why}",
                            row.name, tool.remote_name
                        );
                        continue;
                    }
                }
                let spec = mcp_tool_spec(&row.name, tool);
                let handler = mcp_handler(row.clone(), tool.remote_name.clone(), user_key);
                self.register(spec, handler);
            }
        }
        Ok(self)
    }
}

/// 这次对话的**基线工具表**：内置工具 + 已启用的 SKILL。
///
/// `api_extensions` 要在不构造完整 registry 的前提下知道「哪些名字已被占住」，
/// 才能与 `with_mcp_tools` 判出**同一个**结果。两处各判一次的话，
/// 界面上「挂了几个」与模型「真能调几个」迟早漂 —— 而这正是本项目最不能出的错
/// （与 `skill_visibility` 存在的理由完全一样）。
pub async fn baseline_specs(
    app: Arc<crate::state::AppState>,
    db: &crate::db::DbBridge,
    uid: quill_adapters::UserId,
    skill_root: &std::path::Path,
) -> Result<Vec<ToolSpec>, String> {
    Ok(ToolRegistry::builtin(app, uid)
        .with_skills(db, uid, skill_root)
        .await?
        .specs())
}

/// 把 `UserId` 编成给并发闸门当键用的字符串。
///
/// 带用户是为了不让两个用户各自配的**同名**服务器共用一个闸门 ——
/// 那等于让 B 的调用数去限制 A，而界面上谁也看不出来。
pub fn user_key(uid: quill_adapters::UserId) -> String {
    uid.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------- MCP 挂载口径

/// 挂上去的工具名最长多少字符。
///
/// 64 是本地 4B 模型经 OpenAI 兼容端点转成函数名时的实际上限。超长的名字有两种
/// 死法：有的后端直接拒掉**整次请求**，回一句与你的工具毫无关系的 400；
/// 有的截断，而截断之后两个不同的工具可能撞成同一个名字。
pub const MAX_MCP_TOOL_NAME: usize = 64;

/// 挂载名 = `{服务器}__{工具}`，中间是**两个**下划线。
///
/// **为什么是两个而不是一个。** 服务器名被 `mcp_repo::normalize_name` 归一成
/// `[a-z0-9-]`，**永远不含下划线**，所以 `__` 既是合法的分隔符又是唯一的：
/// 挂载名与一个 SKILL 同名在结构上就不可能（`skills.name` 的 CHECK 是
/// `NOT GLOB '*[^a-z0-9-]*'`，连下划线都不许），与内置工具
/// （`list_experts`）也不可能撞。用单下划线就说不清了：工具名里本来就有
/// `_`，于是服务器 `a` 的工具 `b_c` 与（假想的）服务器 `a_b` 的工具 `c`
/// 会拼成同一个名字，谁顶掉谁都说不清。
///
/// **原名必须一起留着。** 用户在自己的 MCP 配置里认的是 `read-file`，
/// 界面上只显示 `filesystem__read-file` 他会认不出来。所以工具描述里带原名，
/// `GET /api/extensions/mcp` 也把映射报出来。
///
/// `None` = 这个名字归一不出来（服务器给的名字里一个能用的字母数字都没有）。
pub fn mcp_tool_name(server: &str, remote_name: &str) -> Option<String> {
    // 远端名字允许的字符比模型侧宽（有些服务器会带点、冒号、甚至中文），
    // 统一压成 `-` 再收窄到 `[A-Za-z0-9_-]`。
    let head = sanitize(server);
    let tail = sanitize(remote_name);
    // 两段都得有内容才有意义。`server` 走的是 `normalize_name`，正常路径下
    // 不可能为空；这里仍要判，是因为「拼出一个 `__read-file`」这种名字会让
    // 「这台工具属于哪台服务器」在界面上消失 —— 那是拼出来的，不是真的。
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    Some(truncate_to_limit(format!("{head}__{tail}")))
}

/// 压到模型侧能接受的那一套字符，并合并连续的连字符。
fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
        } else if c == '-' {
            // 连续的 `-` 合成一个，免得 `a--b` 与 `a-b` 拼出两个不同却等价的
            // 名字，然后其中一个被「同名顶掉」那条守卫静默跳过。
            if !last_dash && !out.is_empty() {
                out.push('-');
                last_dash = true;
            }
        }
        // 其它字符（点、冒号、空格、中文…）直接丢掉，不留分隔 ——
        // 留着会让 `a.b` 与 `ab` 归一成同一个名字。
    }
    out.trim_matches('-').to_string()
}

/// 超长时截断，并补一小段摘要。
///
/// **必须补摘要。** 光截断的话，`a…很长的名字1` 与 `a…很长的名字2` 会变成同一个，
/// 于是第二个工具被「同名顶掉」这条守卫静默跳过 —— 用户看到的是「少了一个工具」，
/// 而真正的原因（两个名字太长）一个字也没提。补上摘要之后截断仍然是确定的：
/// 同样的输入永远得到同样的名字。
fn truncate_to_limit(name: String) -> String {
    if name.len() <= MAX_MCP_TOOL_NAME {
        return name;
    }
    let digest = crate::db::digest32("mcp.tool_name", &[name.as_bytes()]);
    let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    // 留 9 个字符给 `-` 加 8 位摘要。
    let keep = MAX_MCP_TOOL_NAME - 9;
    let head: String = name.chars().take(keep).collect();
    format!("{head}-{hex}")
}

/// 这次对话里，模型到底看不看得见这个 MCP 工具。
///
/// 与 `SkillVisibility` 同一个形状、同一个理由：界面上要报的是「模型看不看得见」，
/// 不是「服务器报了几个」。判断走 `mcp_tool_visibility` ——
/// 与 `ToolRegistry::with_mcp_tools` **同一个函数**，两边各判一次必然漂。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolVisibility {
    /// 已挂进工具表，模型看得见、也能调。
    Visible,
    /// 想挂却没挂上，原因是给用户看的白话。
    NotMounted(&'static str),
}

impl McpToolVisibility {
    pub fn model_can_see(&self) -> bool {
        matches!(self, McpToolVisibility::Visible)
    }
}

/// 判断走的是**挂上去叫什么**，不是远端原名 —— 因为「同名顶掉」这件事
/// 发生在挂载名上。拿原名去比会漏掉「原名不同、归一后相同」的那一类。
pub fn mcp_tool_visibility(
    existing: &[ToolSpec],
    server: &str,
    tool: &crate::mcp_client::RemoteTool,
) -> McpToolVisibility {
    match mcp_tool_name(server, &tool.remote_name) {
        None => McpToolVisibility::NotMounted("服务器给的名字里没有能用的字母或数字，归一后是空串"),
        Some(name) => {
            if existing.iter().any(|s| s.name == name) {
                // `register` 是「同名替换」：顶掉之后模型看到的是 MCP 的工具，
                // 前面挂好的那个静默消失 —— 这个后果比「这个工具不生效」难查得多。
                // 报原名是因为那才是用户在服务器配置里认的东西。
                return McpToolVisibility::NotMounted("挂载名与已有工具相同，挂进去会顶掉它");
            }
            if !is_object_schema(&tool.input_schema) {
                // 不替它补一个空 schema —— 补了就是编一份「这个工具没有参数」的事实，
                // 而服务器明明报了别的东西。模型照着错的 schema 传参，症状会出现在
                // 工具执行阶段，离真正的病因很远。
                return McpToolVisibility::NotMounted(
                    "服务器给的 inputSchema 不是 object 类型，模型无从得知该传什么参数",
                );
            }
            McpToolVisibility::Visible
        }
    }
}

/// MCP 规范要求 `inputSchema` 是 object。**判得宽一点**：只认「明确说了不是
/// object」的情况；缺 `type` 的按 object 放行 —— 有一批真实服务器不写 `type`，
/// 而它们的参数是能用的，为这个把它们全踢掉是白丢能力。
fn is_object_schema(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(t)) => t == "object",
        None => true,
        Some(_) => false,
    }
}

/// MCP 工具的 `ToolSpec`。**schema 原样带过去**，不重写、不补默认值。
pub fn mcp_tool_spec(server: &str, tool: &crate::mcp_client::RemoteTool) -> ToolSpec {
    let name = mcp_tool_name(server, &tool.remote_name).unwrap_or_else(|| tool.remote_name.clone());
    // 描述里带上服务器与原名：模型据此知道这个工具的来路，用户也能在界面上
    // 把 `filesystem__read-file` 对回自己配置里的 `read-file`。
    let origin = format!(
        "（来自 MCP 服务器 {server}，工具原名 {}）",
        tool.remote_name
    );
    let description = if tool.description.trim().is_empty() {
        // 服务器没写描述就说没写。补一句「暂无描述」比留空好：
        // 留空的话模型只能从 schema 猜，而空描述在很多端点上会被整条丢掉。
        format!("{origin} 服务器没有为这个工具写描述，请按下面的参数说明判断要不要用。")
    } else {
        format!("{}\n{origin}", tool.description.trim())
    };
    ToolSpec::new(name, description).with_parameters(tool.input_schema.clone())
}

/// MCP 工具的执行体。**每次调用重新握手**，理由见 `mcp_client::call_tool`。
fn mcp_handler(
    row: crate::mcp_repo::McpServerRow,
    remote_name: String,
    user_key: &str,
) -> ToolHandler {
    let user_key = user_key.to_string();
    Arc::new(move |args: &Value| {
        crate::mcp_client::call_tool_blocking(&row, &user_key, &remote_name, args)
    })
}

/// 模型在这次对话里看不看得见这个 SKILL。
///
/// 界面上要报的是**这个**，不是「库里有没有这一行」。`GET /api/extensions/skills`
/// 与 `with_skills` 调的是同一个函数：两边各判一次，迟早会漂，漂了就变成
/// 「界面显示能用、模型那边根本没有」——那正是本项目最不能出的错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillVisibility {
    /// 已挂进工具表，模型看得见、也能调。
    Visible,
    /// 用户自己关掉的。不是故障，界面上照实说「已停用」。
    Disabled,
    /// 想挂却没挂上，原因是给用户看的白话。
    NotMounted(&'static str),
}

impl SkillVisibility {
    /// 界面上那个「模型看得见」的布尔值。
    pub fn model_can_see(&self) -> bool {
        matches!(self, SkillVisibility::Visible)
    }
}

/// 这次对话里，模型到底看不看得见这个 SKILL。
///
/// `existing` 是**已经占住**的工具名（内置工具 + 前面已挂上的 SKILL）。
/// 挑出**启用**的 MCP 服务器。抽成独立函数是为了能单测 ——
/// `with_mcp_tools` 本体要真起 stdio 进程，测不了「过滤」这一层，
/// 而「停用之后到底还挂不挂」恰恰是最该被钉住的那一条（ISSUE-026）。
pub fn enabled_servers(
    rows: Vec<crate::mcp_repo::McpServerRow>,
) -> Vec<crate::mcp_repo::McpServerRow> {
    rows.into_iter().filter(|r| r.enabled).collect()
}

pub fn skill_visibility(
    existing: &[ToolSpec],
    row: &crate::skills_repo::SkillRow,
    body: &str,
) -> SkillVisibility {
    if !row.enabled {
        return SkillVisibility::Disabled;
    }
    match veto(existing, row, body) {
        None => SkillVisibility::Visible,
        Some(why) => SkillVisibility::NotMounted(why),
    }
}

/// 这个 SKILL 能不能挂进工具表。`Some(原因)` = 不挂。
///
/// 提成独立函数是因为这两条否决路径**没法靠真实数据走到**：0001 迁移里
/// `skills.name` 的 CHECK 是 `NOT GLOB '*[^a-z0-9-]*'`，连下划线都不允许，
/// 而内置工具叫 `list_experts`。所以「同名顶掉内置」在今天的 schema 下是不可达
/// 的 —— 但 MCP 工具名常用连字符（`read-file`），那条路一通，这个守卫就有用了。
/// 放在 `with_skills` 里就地判断的话，就只能写成一条跑不到的分支。
///
/// 收的是 `existing: &[ToolSpec]` 而不是 `&ToolRegistry`，是为了让
/// `api_extensions::list_skills` 在**不构造 registry** 的前提下复用同一条判断 ——
/// 两处各写一份是这个项目最容易出的错（界面上说一套、`with_skills` 做另一套）。
fn veto(
    existing: &[ToolSpec],
    row: &crate::skills_repo::SkillRow,
    body: &str,
) -> Option<&'static str> {
    if body.trim().is_empty() {
        return Some("库里有行但磁盘上没有正文（文件被删了，或目录没挂上）");
    }
    if existing.iter().any(|s| s.name == row.name) {
        // `register` 是「同名替换」：顶掉之后模型看到的是 SKILL 的方法，
        // 内置工具静默消失，这个后果比「这个技能不生效」难查得多。
        return Some("与已有工具同名，挂进去会顶掉它");
    }
    None
}

/// SKILL 工具的执行体。
///
/// **调用时才回灌正文。** 工具描述里只放摘要（见 `skills_repo::as_tool_spec`），
/// 否则每个启用技能的全文都是每轮请求的常驻开销，装十几个技能就会顶爆上下文
/// 窗口 —— 实测 13 个技能 32430 字符 ≈ 10810 tokens > 8192，连「1+1」都必然
/// 503。见 ISSUE-036。
///
/// 正文真的超了 `MAX_RESULT_CHARS` 时**明说被截断了**，不能悄悄给半份：
/// 模型会以为方法只写了一半，然后照着残缺的方法去做事。
/// `get_expert_detail` 的名字匹配。
///
/// **两个内置工具曾经对不上**：`list_experts` 的返回值是「显示名（id）」这种形状，
/// 而 `get_expert_detail` 的参数说明偏偏写着「取自 `list_experts` 的返回值」——
/// 模型于是把整串「成本分析师（cost-analyst）」传回来，而匹配只认纯 `display_name`，
/// 于是**必然失败**。实测就是模型在 `list_experts` 与 `get_expert_detail` 之间
/// 空转到 4 轮上限（实跑第 5、7 条任务），报成 `tool_loop_exhausted`，
/// 看起来像模型弱，其实是自己两个工具的契约错位。见 ISSUE-040。
///
/// 抽成独立纯函数是为了能单测：真起 AppState 要数据库与配置。
pub fn matches_expert_query(id: &str, display_name: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if q.eq_ignore_ascii_case(id) || q.eq_ignore_ascii_case(display_name) {
        return true;
    }
    // 「显示名（id）」原样传回来的情形
    if let Some((name, _inner_id)) = q.rsplit_once('（') {
        let name = name.trim().trim_end_matches('）').trim();
        if !name.is_empty() && name.eq_ignore_ascii_case(display_name) {
            return true;
        }
    }
    false
}

fn skill_handler(name: &str, body: &str) -> ToolHandler {
    let name = name.to_string();
    let body = body.to_string();
    Arc::new(move |args: &Value| {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or("缺少参数 task —— 请写清要交给这套方法处理的具体任务。")?;
        let (text, note) = if body.chars().count() > MAX_RESULT_CHARS {
            let kept: String = body.chars().take(MAX_RESULT_CHARS).collect();
            (
                kept,
                format!(
                    "\n\n（这套方法正文共 {} 字符，超过单次回灌上限 {MAX_RESULT_CHARS} 字符，\
                     上面只是前半部分。后半段请直接读技能文件，或把任务拆小再调一次。）",
                    body.chars().count()
                ),
            )
        } else {
            (body.clone(), String::new())
        };
        Ok(format!(
            "已加载「{name}」这套方法。完整正文如下：\n\n{text}{note}\n\n\
             现在按这套方法处理这个任务：\n{task}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> ToolCall {
        ToolCall::new("call-1", name, json!({}))
    }

    fn with_one_tool() -> ToolRegistry {
        let mut r = ToolRegistry {
            specs: Vec::new(),
            handlers: Vec::new(),
        };
        r.register(
            ToolSpec::new("echo", "回显"),
            Arc::new(|args: &Value| {
                Ok(args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string())
            }),
        );
        r
    }

    /// 内置表里挂了 AppState 的闭包，单测里造一个够用的替身即可 ——
    /// 这里只断言「结构完整」，不碰数据库。
    fn dummy_app() -> Arc<crate::state::AppState> {
        Arc::new(crate::state::AppState {
            config: crate::config::Config::from_env(),
            tokens: Arc::new(crate::auth::EnvTokenResolver::default()),
            db: None,
            db_problem: Some("测试注入：未建库".to_string()),
            llm: Arc::new(std::sync::RwLock::new(None)),
            llm_config: Arc::new(std::sync::RwLock::new(Default::default())),
            providers: Arc::new(std::sync::RwLock::new(Default::default())),
            login_limiter: Arc::new(crate::ratelimit::RateLimiter::default()),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        })
    }

    fn dummy_builtin() -> ToolRegistry {
        ToolRegistry::builtin(dummy_app(), quill_adapters::UserId::from_bytes([7u8; 16]))
    }

    #[test]
    fn a_builtin_registry_is_not_empty_and_every_spec_has_a_handler() {
        let r = dummy_builtin();
        assert!(!r.is_empty(), "内置工具表不该是空的");
        for spec in r.specs() {
            assert!(
                r.handler(&spec.name).is_some(),
                "工具 {} 有定义但没有执行体 —— 模型会调用到一个必然失败的东西",
                spec.name
            );
        }
    }

    #[test]
    fn every_tool_spec_declares_a_json_schema_object() {
        // 不带 parameters 的工具：模型无从得知该传什么参数，只能瞎猜。
        for spec in dummy_builtin().specs() {
            assert_eq!(
                spec.parameters.get("type").and_then(Value::as_str),
                Some("object"),
                "工具 {} 的 parameters 必须是 object 类型",
                spec.name
            );
            assert!(
                spec.parameters.get("properties").is_some(),
                "工具 {} 的 parameters 缺少 properties",
                spec.name
            );
        }
    }

    #[test]
    fn an_unknown_tool_is_reported_to_the_model_instead_of_being_silently_dropped() {
        // 模型幻觉出一个不存在的工具时，必须把事实告诉它。
        // 静默跳过会让模型反复尝试同一个不存在的工具。
        let r = with_one_tool();
        let err = r.call(&call("nope")).expect_err("未知工具必须报错");
        assert!(err.contains("nope"), "错误要说明是哪个工具：{err}");
        assert!(err.contains("echo"), "错误要列出当前可用的工具：{err}");
    }

    #[test]
    fn tool_arguments_reach_the_handler() {
        let r = with_one_tool();
        let c = ToolCall::new("c1", "echo", json!({ "text": "你好" }));
        assert_eq!(r.call(&c).expect("应成功"), "你好");
    }

    #[test]
    fn a_failed_call_is_still_rendered_for_the_model() {
        let r = with_one_tool();
        let text = r.render_result(&call("nope"), r.call(&call("nope")));
        assert!(text.contains("工具执行失败"), "失败要如实标记：{text}");
    }

    #[test]
    fn an_empty_success_is_not_rendered_as_a_bare_empty_string() {
        // 空字符串回灌给模型，它会以为工具什么都没查到。
        let r = with_one_tool();
        let text = r.render_result(&call("echo"), Ok("   ".to_string()));
        assert!(text.contains("没有返回内容"), "空结果要说明原因：{text}");
    }

    #[test]
    fn an_oversized_result_is_truncated_and_says_so() {
        // 截断必须明说，否则模型会以为看到的是全部，据此给出错误结论。
        let r = with_one_tool();
        let huge = "字".repeat(MAX_RESULT_CHARS + 500);
        let text = r.render_result(&call("echo"), Ok(huge));
        assert!(
            text.contains("已截断"),
            "截断必须告知模型，否则它会以为看到的是全部"
        );
        assert!(
            text.chars().count() < MAX_RESULT_CHARS + 200,
            "截断后长度仍应明显短于原文"
        );
    }

    #[test]
    fn registering_the_same_name_twice_replaces_rather_than_duplicates() {
        let mut r = with_one_tool();
        r.register(
            ToolSpec::new("echo", "新的回显"),
            Arc::new(|_| Ok("新".into())),
        );
        assert_eq!(
            r.specs().iter().filter(|s| s.name == "echo").count(),
            1,
            "同名工具重复注册会让模型在两个定义间无所适从"
        );
        assert_eq!(r.call(&call("echo")).expect("应走新实现"), "新");
    }

    #[test]
    fn the_expert_tools_can_be_left_out_when_the_role_is_already_picked() {
        // ISSUE-041：角色在界面上定过之后，这两个工具对模型就是纯噪音 ——
        // 实测 11 条真实任务里 5 条因此以 tool_loop_exhausted 收场。
        let uid = quill_adapters::UserId::from_bytes([7u8; 16]);
        let app = dummy_app();

        let with = ToolRegistry::builtin_with_expert_tools(Arc::clone(&app), uid, true);
        let without = ToolRegistry::builtin_with_expert_tools(Arc::clone(&app), uid, false);

        let names = |r: &ToolRegistry| {
            r.specs()
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<String>>()
        };
        let (a, b) = (names(&with), names(&without));

        // 这个夹具库里有专家，所以开着的那个应当挂上；
        // 关掉的那个**必须**一个专家工具都不剩。
        for gone in ["list_experts", "get_expert_detail"] {
            if a.iter().any(|n| n == gone) {
                assert!(!b.iter().any(|n| n == gone), "关掉后仍挂上了 {gone}：{b:?}");
            }
        }
        // 别的内置工具一个都不能少 —— 不能为了省预算把工具表掏空。
        assert_eq!(
            b.len(),
            a.len().saturating_sub(
                a.iter()
                    .filter(|n| *n == "list_experts" || *n == "get_expert_detail")
                    .count()
            ),
            "关掉专家工具只该少这两个：{a:?} vs {b:?}"
        );
    }

    #[test]
    fn the_final_answer_prompt_asks_for_a_honest_partial_not_a_guess() {
        // 收尾调用的措辞是关键：要求它「说出查到了什么、缺什么」，
        // 而不是「想办法给出答案」—— 后者只会让它继续要工具。
        let p = FINAL_ANSWER_PROMPT;
        assert!(
            p.contains("不要再调用任何工具"),
            "要先断掉调工具的念头：{p}"
        );
        assert!(p.contains("查到了什么"), "要它把已有的说出来：{p}");
        assert!(p.contains("拿不到"), "要它说清缺什么：{p}");
        // 反编造：这几条是这个提示词存在的理由，少一条都不行。
        for anti in ["不要给估计值", "不要凭印象填", "可能是编的"] {
            assert!(p.contains(anti), "收尾提示里不该少「{anti}」：{p}");
        }
        // 不能出现「必须给出正确答案」这类要求 —— 那是查不到数据时的死路。
        for forbidden in ["必须给出正确答案", "想办法", "一定"] {
            assert!(
                !p.contains(forbidden),
                "收尾提示里不该出现「{forbidden}」：{p}"
            );
        }
    }

    #[test]
    fn the_final_answer_marker_is_not_something_a_model_can_call() {
        // 它只出现在工具轨迹里，不是可调用的工具名 —— 不能和真工具重名。
        assert_eq!(FINAL_ANSWER_MARKER, "final_answer");
        assert_ne!(FINAL_ANSWER_MARKER, "list_experts");
        assert_ne!(FINAL_ANSWER_MARKER, "notes__read-note");
    }

    #[test]
    fn an_expert_lookup_accepts_exactly_what_list_experts_hands_back() {
        // ISSUE-040：list_experts 返回「显示名（id）」，参数说明也让模型原样传回，
        // 所以这三种形状**必须**都能命中，否则模型会在两个工具之间空转到轮次上限。
        for q in [
            "成本分析师（cost-analyst）",     // 原样返回值
            "成本分析师",                     // 只传显示名
            "cost-analyst",                   // 只传 id
            "  成本分析师（cost-analyst）  ", // 带空白
        ] {
            assert!(
                matches_expert_query("cost-analyst", "成本分析师", q),
                "「{q}」应当命中"
            );
        }
    }

    #[test]
    fn an_expert_lookup_still_refuses_a_name_that_is_not_there() {
        // 不能为了上面那条把匹配放得过宽：认不出来就得说认不出来。
        assert!(!matches_expert_query("cost-analyst", "成本分析师", ""));
        assert!(!matches_expert_query("cost-analyst", "成本分析师", "   "));
        assert!(!matches_expert_query(
            "cost-analyst",
            "成本分析师",
            "收入分析师"
        ));
        assert!(!matches_expert_query(
            "cost-analyst",
            "成本分析师",
            "（cost-analyst）"
        ));
    }

    #[test]
    fn a_skill_tool_result_returns_the_full_body_so_the_model_can_follow_it() {
        // 修之前的形态是「只回执，正文常驻在工具描述里」。那样每个启用技能的
        // 全文都是每轮请求的固定开销，13 个技能就能把 8192 的窗口顶爆，
        // 连「1+1」都必然 503（ISSUE-036）。现在改成调用时才回灌正文。
        let body = "第一步：读文件。第二步：写下拉。第三步：跑测试。";
        let h = skill_handler("code-review", body);
        let out = h(&json!({ "task": "审一下 x.rs 里的下拉" })).expect("应成功");
        assert!(out.contains("code-review"), "要说清用的是哪套方法：{out}");
        assert!(
            out.contains("审一下 x.rs 里的下拉"),
            "任务要回给模型：{out}"
        );
        assert!(
            out.contains("第三步：跑测试。"),
            "正文必须整段回灌，否则模型拿不到方法：{out}"
        );
    }

    #[test]
    fn a_skill_body_too_long_for_one_refeed_says_it_was_cut_instead_of_quietly_truncating() {
        // 悄悄给半份正文，模型会以为方法只写了一半，然后照着残缺的方法做事。
        let body = "字".repeat(MAX_RESULT_CHARS + 500);
        let out = skill_handler("huge", &body)(&json!({ "task": "干活" })).expect("应成功");
        assert!(
            out.contains("超过单次回灌上限"),
            "超长必须明说，不能悄悄截断：{}",
            &out[out.len().saturating_sub(200)..]
        );
    }

    #[test]
    fn a_skill_tool_called_without_a_task_is_reported_back_to_the_model() {
        for args in [json!({}), json!({"task": "  "}), json!({"task": 7})] {
            let err = skill_handler("x", "正文")(&args).expect_err("缺 task 必须报错");
            assert!(err.contains("task"), "要说清缺哪个参数：{err}");
        }
    }

    fn skill_row_named(name: &str) -> crate::skills_repo::SkillRow {
        crate::skills_repo::SkillRow {
            name: name.to_string(),
            version: "0.1.0".into(),
            source: "local".into(),
            source_ref: None,
            description: String::new(),
            enabled: true,
            install_path: "/tmp/x".into(),
            tool_allowlist: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_skill_with_no_body_on_disk_is_vetoed_rather_than_registered_empty() {
        // 挂一个 description 为空的工具，模型会调到一个必然没有产出的东西，
        // 而界面上这个技能还显示「已启用」。
        let specs = with_one_tool().specs();
        for body in ["", "   ", "\n\t "] {
            let why = veto(&specs, &skill_row_named("gone"), body).expect("正文为空必须否决");
            assert!(why.contains("正文"), "要说清是正文没了：{why}");
        }
        assert_eq!(veto(&specs, &skill_row_named("gone"), "有正文"), None);
    }

    #[test]
    fn a_skill_may_not_replace_a_tool_that_already_has_that_name() {
        // `register` 是「同名替换」。真走到这一步，内置工具会静默消失 ——
        // 模型看到的是 SKILL 的方法，界面上却还显示着原来的工具。
        let mut r = with_one_tool();
        let specs = r.specs();
        let why =
            veto(&specs, &skill_row_named("echo"), "一套叫 echo 的方法").expect("同名必须否决");
        assert!(why.contains("顶掉"), "要说清后果：{why}");
        r.register(
            ToolSpec::new("read-file", "模拟一个 MCP 工具"),
            Arc::new(|_| Ok(String::new())),
        );
        assert!(
            veto(&r.specs(), &skill_row_named("read-file"), "方法").is_some(),
            "MCP 工具名常用连字符，那条路一通这个守卫就要真的生效"
        );
    }

    #[test]
    fn a_disabled_skill_is_reported_as_disabled_not_as_a_mounting_failure() {
        // 停用是用户的选择，界面上要说「已停用」。混进「没挂上，原因是正文没了」
        // 那一类，会让用户去查一个根本不存在的问题。
        let mut row = skill_row_named("off");
        row.enabled = false;
        let specs = with_one_tool().specs();
        assert_eq!(
            skill_visibility(&specs, &row, "有正文"),
            SkillVisibility::Disabled
        );
        assert!(!skill_visibility(&specs, &row, "有正文").model_can_see());
    }

    /// ISSUE-026 的回归：**停用的 MCP 服务器不许进这一轮的工具表。**
    ///
    /// 修之前 `with_mcp_tools` 一行都没判 `enabled` —— 用户在配置里关掉了
    /// 一台服务器，它的工具照样出现在模型的工具表里，界面上也看不出差别。
    /// 与 ISSUE-008（`tool_allowlist` 只存不用）同一类病。
    fn mcp_row(name: &str, enabled: bool) -> crate::mcp_repo::McpServerRow {
        crate::mcp_repo::McpServerRow {
            name: name.into(),
            transport: "stdio".into(),
            command: Some("echo".into()),
            args: vec![],
            env: vec![],
            url: None,
            headers: vec![],
            enabled,
            timeout_ms: 30_000,
            description: String::new(),
            cwd: None,
            max_concurrent_calls: None,
            enabled_capabilities: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_disabled_mcp_server_is_not_mounted_at_all() {
        let rows = vec![
            mcp_row("on", true),
            mcp_row("off", false),
            mcp_row("also-on", true),
        ];
        let kept: Vec<String> = enabled_servers(rows).into_iter().map(|r| r.name).collect();
        assert_eq!(
            kept,
            vec!["on".to_string(), "also-on".to_string()],
            "停用的那台不许进工具表"
        );
    }

    #[test]
    fn every_server_disabled_leaves_nothing_to_mount() {
        let kept = enabled_servers(vec![mcp_row("a", false), mcp_row("b", false)]);
        assert!(
            kept.is_empty(),
            "全停用时不该再去握手 —— 那会把用户主动关掉的进程全拉起来一遍"
        );
    }

    #[test]
    fn no_server_configured_stays_empty() {
        assert!(enabled_servers(vec![]).is_empty());
    }

    #[test]
    fn a_mounted_skill_reports_itself_visible_and_an_unmountable_one_says_why() {
        let specs = with_one_tool().specs();
        let ok = skill_visibility(&specs, &skill_row_named("code-review"), "一套方法");
        assert_eq!(ok, SkillVisibility::Visible);
        assert!(ok.model_can_see(), "挂了就是看得见，界面上不能显示成没挂");

        let why = match skill_visibility(&specs, &skill_row_named("code-review"), "  ") {
            SkillVisibility::NotMounted(w) => w,
            other => panic!("正文为空必须是「没挂上」，实际：{other:?}"),
        };
        assert!(why.contains("正文"), "要把原因说给用户听：{why}");
        assert!(
            !skill_visibility(&specs, &skill_row_named("code-review"), "  ").model_can_see(),
            "挂不上就不许显示成模型看得见"
        );
    }
}
