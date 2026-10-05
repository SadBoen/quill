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
        let mut r = ToolRegistry {
            specs: Vec::new(),
            handlers: Vec::new(),
        };

        // 两个闭包各自持有一份 Arc：AppState 里的 DbBridge/LLM 都是 Arc，
        // 克隆是廉价的引用计数递增，不会复制任何运行状态。
        let app2 = Arc::clone(&app);

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
                        "description": "专家名称，取自 list_experts 的返回值。"
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
                    .find(|e| e.display_name().eq_ignore_ascii_case(&name))
                    .ok_or(format!("没有名为「{name}」的专家。"))?;
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
            if !row.enabled {
                continue;
            }
            let body =
                crate::api_extensions::read_skill_body(&root.join(format!("{}.md", row.name)));
            if body.trim().is_empty() {
                eprintln!(
                    "[tools] 跳过 SKILL {}：库里有行但磁盘上没有正文（文件被删了，或目录没挂上）",
                    row.name
                );
                continue;
            }
            if self.specs.iter().any(|s| s.name == row.name) {
                // `register` 是「同名替换」。一个叫 `list_experts` 的 SKILL 会
                // 悄悄顶掉内置工具 —— 那个后果比「这个技能不生效」难查得多。
                eprintln!(
                    "[tools] 跳过 SKILL {}：与已有工具同名，挂进去会顶掉它",
                    row.name
                );
                continue;
            }
            let spec = crate::skills_repo::as_tool_spec(&row, &body);
            let handler = skill_handler(&row.name);
            self.register(spec, handler);
        }
        Ok(self)
    }
}

/// SKILL 工具的执行体。
///
/// **只回执，不回正文** —— 正文已经在 `as_tool_spec` 里进了工具描述，而工具
/// 描述每轮请求都带着。再回一份就是同一段文字读两遍；更糟的是正文可能超过
/// `MAX_RESULT_CHARS`，于是回灌给模型的是一份**被截断的**副本，模型会误以为
/// 方法只写了一半。
fn skill_handler(name: &str) -> ToolHandler {
    let name = name.to_string();
    Arc::new(move |args: &Value| {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or("缺少参数 task —— 请写清要交给这套方法处理的具体任务。")?;
        Ok(format!(
            "已加载「{name}」这套方法（正文见该工具的描述）。\
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
    fn dummy_builtin() -> ToolRegistry {
        ToolRegistry::builtin(
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
            }),
            quill_adapters::UserId::from_bytes([7u8; 16]),
        )
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
    fn the_round_trip_budget_is_bounded() {
        // 不设上限的话，一个「不断调用工具」的循环能把请求挂到超时，
        // 且每一轮都在烧 CPU。
        assert!(MAX_TOOL_ROUNDS > 0, "上限不能是 0，否则工具永远不执行");
        assert!(
            MAX_TOOL_ROUNDS <= 8,
            "上限 {} 太大：每轮都是一次真实模型调用",
            MAX_TOOL_ROUNDS
        );
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
        assert!(text.contains("已截断"), "截断必须告知模型，否则它会以为看到的是全部");
        assert!(
            text.chars().count() < MAX_RESULT_CHARS + 200,
            "截断后长度仍应明显短于原文"
        );
    }

    #[test]
    fn registering_the_same_name_twice_replaces_rather_than_duplicates() {
        let mut r = with_one_tool();
        r.register(ToolSpec::new("echo", "新的回显"), Arc::new(|_| Ok("新".into())));
        assert_eq!(
            r.specs().iter().filter(|s| s.name == "echo").count(),
            1,
            "同名工具重复注册会让模型在两个定义间无所适从"
        );
        assert_eq!(r.call(&call("echo")).expect("应走新实现"), "新");
    }

    #[test]
    fn a_skill_tool_result_reports_the_task_and_does_not_repeat_the_body() {
        let h = skill_handler("code-review");
        let out = h(&json!({ "task": "审一下 x.rs 里的下拉" })).expect("应成功");
        assert!(out.contains("code-review"), "要说清用的是哪套方法：{out}");
        assert!(out.contains("审一下 x.rs 里的下拉"), "任务要回给模型：{out}");
        assert!(
            out.chars().count() < MAX_RESULT_CHARS,
            "回执必须短到不被 render_result 截断，否则模型会拿到半截方法"
        );
    }

    #[test]
    fn a_skill_tool_called_without_a_task_is_reported_back_to_the_model() {
        for args in [json!({}), json!({"task": "  "}), json!({"task": 7})] {
            let err = skill_handler("x")(&args).expect_err("缺 task 必须报错");
            assert!(err.contains("task"), "要说清缺哪个参数：{err}");
        }
    }
}
