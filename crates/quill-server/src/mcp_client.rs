//! MCP 协议层：**真的**把 stdio 服务器拉起来，真的走一遍 `initialize` + `tools/list`。
//!
//! 分工是硬的：`mcp_repo` 只管存配置（它自己的模块文档里写明了「这里不连服务器」），
//! 本模块是**唯一**发起连接的地方。两者混在一起的后果是「保存配置会因为连不上而失败」，
//! 而连不上往往正是用户需要先把配置登记进去才能排查的东西。
//!
//! **只铺了 stdio。** `streamable_http` / `sse` 明确报「还没铺」，不假装连过：
//! `probe` 对这两种传输给 `probed=false` 加一句白话原因。报一个「没做」比报一个
//! 编出来的 `connected: false` 强 —— 后者会让用户去查一个根本不存在的网络故障。
//!
//! **命令是发起请求那个用户自己配的。** `mcp_servers` 按 `user_id` 过滤，
//! 拉起谁的进程由谁的配置决定；这里不做任何跨用户兜底，也不接受请求体里
//! 直接塞进来的命令 —— 命令只能来自 `mcp_repo::list` 的读结果。
//!
//! **失败必须说得清。** 握手失败、超时、服务器起不来，三者的下一步完全不同
//! （改配置 / 放宽超时 / 装依赖），所以每条错误都带上一条能照着做的建议。

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    ClientCapabilities, ClientConfig, Implementation, PaginatedRequestParams, Tool as McpTool,
};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::mcp_repo::McpServerRow;

/// 单台服务器本轮探测的结果。字段全是**实测**出来的，没有一个是推断的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// 对应 `mcp_servers.name`。
    pub name: String,
    /// 本轮是不是真的发起过协议握手。停用 / 非 stdio / 缺 command 都是 false。
    pub probed: bool,
    /// 真的完成 `initialize` 且 `tools/list` 拿到了结果。
    pub connected: bool,
    /// 按 `enabled_capabilities` 过滤之后，真正会交给模型用的工具条数。
    pub tool_count: usize,
    /// 探测不到时的白话原因。`connected` 为真时是 `None`。
    pub error: Option<String>,
    /// 服务器自报的协议版本，例如 `2025-06-18`。
    pub protocol_version: Option<String>,
    /// 服务器自报的实现名与版本，例如 `filesystem-server 1.2.0`。
    pub server_info: Option<String>,
}

impl Probe {
    fn not_probed(row: &McpServerRow, why: impl Into<String>) -> Self {
        Probe {
            name: row.name.clone(),
            probed: false,
            connected: false,
            tool_count: 0,
            error: Some(why.into()),
            protocol_version: None,
            server_info: None,
        }
    }

    fn failed(row: &McpServerRow, why: impl Into<String>) -> Self {
        Probe {
            probed: true,
            error: Some(why.into()),
            ..Probe::not_probed(row, "")
        }
    }
}

/// 一台 MCP 服务器 `tools/list` 返回的单个工具。**原样保留**服务器给的名字与
/// schema：改名的代价是用户在自己的配置里也认不出这个工具了。
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub remote_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 探测预算的上限。配置里的 `timeout_ms` 最大 600 秒，但那是**调用工具**的预算；
/// 「打开设备页列一下有哪些服务器」不该等十分钟。上限取 10 秒：本地 stdio 进程
/// 起个来回给 10 秒足够起不来的那类问题（缺依赖、端口占满、进程立刻崩）也够暴露了。
pub const PROBE_BUDGET_CEILING_MS: i64 = 10_000;

/// 能力名。MCP 规范里 `tools` / `resources` / `prompts` 三选其一或全开。
const CAP_TOOLS: &str = "tools";

/// `enabled_capabilities` 三态里，「tools 类的工具会不会交给模型」。
///
/// `None`=全禁、`Some([])`=全开、`Some(v)`=精确列举。**必须与 `mcp_repo::asset_hash`
/// 的三态口径一致** —— 那里把三态编进指纹，这里决定 `tool_count`，两边漂了就会出现
/// 「配置没变但工具数变了」。
pub fn tools_capability_on(row: &McpServerRow) -> bool {
    match &row.enabled_capabilities {
        None => false,
        // `Some(vec![])` 是**全开**，不是「一个都没列出来所以一个都不给」。
        // 早先这里直接 `v.iter().any(...)`，空数组的 `any` 恒为 false ——
        // 于是能力全开的服务器会被报成 0 个工具，而且界面上看不出是哪里错的。
        // 空与非空必须在这里分叉。
        Some(v) if v.is_empty() => true,
        Some(v) => v.iter().any(|c| c.eq_ignore_ascii_case(CAP_TOOLS)),
    }
}

/// 探测一台服务器。**永远不会 panic、永远返回结构完整的结果** —— 调用方要把它
/// 直接渲染到界面上，缺字段的话前端就得自己猜「那是什么意思」。
pub async fn probe(row: &McpServerRow) -> Probe {
    if !row.enabled {
        return Probe::not_probed(
            row,
            "已停用（你自己关掉的），本轮没有发起任何连接。",
        );
    }
    if row.transport != "stdio" {
        return Probe::not_probed(row, format!(
            "传输方式 {} 的协议层还没铺，本轮只铺了 stdio —— 这不是连接失败，是还没做。",
            row.transport
        ));
    }
    let command = row.command.as_deref().unwrap_or("").trim();
    if command.is_empty() {
        return Probe::not_probed(
            row,
            "stdio 配置里没有 command，没法拉起本地进程。\
             下一步：在这一行填上可执行文件名，例如 `npx`。",
        );
    }

    let budget = probe_budget(row.timeout_ms);
    match tokio::time::timeout(budget, handshake(row)).await {
        Ok(Ok(tools)) => {
            let info = tools.info;
            let tool_count = if tools_capability_on(row) {
                tools.tools.len()
            } else {
                // 连上了、但能力被关掉，报 0 而不是报真实条数：模型看不到的工具
                // 不该被算成「可用的工具」。原因写在 note 里。
                0
            };
            Probe {
                name: row.name.clone(),
                probed: true,
                connected: true,
                tool_count,
                error: None,
                protocol_version: Some(info.0),
                server_info: Some(info.1),
            }
        }
        Ok(Err(why)) => Probe::failed(row, why),
        Err(_) => Probe::failed(
            row,
            format!(
                "握手加 tools/list 在 {} 毫秒内没跑完（已按上限截断）。\
                 下一步：先在命令行里手动跑一次 `{}`，它自己都不回话就别指望协议层能连上。",
                budget.as_millis(),
                command_line(row)
            ),
        ),
    }
}

/// 探测一批。**并发**，且是**并发地去连真实的用户进程**：一台起不起来的服务器
/// 慢 10 秒，不该让另外三台跟着排队 —— 界面上「刷新」是一个动作，不该按台数线性变慢。
///
/// 返回值与 `rows` **同序**：调用方按下标对齐，不用按名字再查一遍（名字已归一，
/// 但按名字查会把「哪台对不上」的错误藏起来）。
pub async fn probe_all(rows: Vec<McpServerRow>) -> Vec<Probe> {
    let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let mut set = tokio::task::JoinSet::new();
    for (idx, row) in rows.into_iter().enumerate() {
        set.spawn(async move { (idx, probe(&row).await) });
    }
    // **先按输入顺序占好位，再按完成顺序填。** `JoinSet::join_next` 是谁先跑完
    // 先给谁，直接 push 的话返回顺序会随机器负载抖动 —— 而调用方是拿这个下标去
    // 对齐 `servers` 的，顺序一乱，「哪台服务器的工具数」就全串位了。
    let mut slots: Vec<Option<Probe>> = names.iter().map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((idx, p)) => slots[idx] = Some(p),
            // 探测任务自己不会 panic（`probe` 内部没有 unwrap）。真出了 join 错误，
            // 也不能让整个接口 500 —— 那是「一个用户进程有问题」变成「整页 MCP 打不开」。
            // 槽位留着 None，下面按名字填成一条「没探测成」的记录。
            Err(e) => eprintln!("[mcp] 探测任务没能跑完：{e}"),
        }
    }
    slots
        .into_iter()
        .zip(names)
        .map(|(slot, name)| slot.unwrap_or_else(|| Probe {
            name,
            probed: false,
            connected: false,
            tool_count: 0,
            error: Some("探测任务没能跑完（见服务端日志）".to_string()),
            protocol_version: None,
            server_info: None,
        }))
        .collect()
}

fn probe_budget(timeout_ms: i64) -> Duration {
    let ms = timeout_ms.clamp(1_000, PROBE_BUDGET_CEILING_MS);
    Duration::from_millis(ms as u64)
}

struct HandshakeTools {
    tools: Vec<RemoteTool>,
    /// `(协议版本, 实现名 版本)`
    info: (String, String),
}

async fn handshake(row: &McpServerRow) -> Result<HandshakeTools, String> {
    let command = row.command.clone().unwrap_or_default();
    let mut cmd = Command::new(command.trim());
    cmd.args(&row.args);
    if let Some(cwd) = row.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        cmd.current_dir(cwd);
    }
    for (k, v) in &row.env {
        cmd.env(k, v);
    }
    // stdin/stdout 由传输层接管。**stderr 单独接走**：stdio 服务器启动失败的信息
    // 几乎全在 stderr 上，继承到服务端日志的话，用户在界面上只会看到一句
    // 「连不上」——而那句话对「少装一个依赖」和「地址写错」是同一个答案。
    let (transport, child_stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_hint(row, e))?;
    let stderr: Arc<StderrTail> = Arc::new(StderrTail::default());
    // 必须有人**同时**在读 stderr：不读的话管道写满（Linux 64KB）之后子进程会
    // 阻塞在 write 上，握手就永远等不到 —— 表现为「每台服务器都超时」。
    let drain = child_stderr.map(|mut s| {
        let sink = Arc::clone(&stderr);
        tokio::spawn(async move {
            let mut raw = Vec::new();
            let _ = s.read_to_end(&mut raw).await;
            sink.set(&String::from_utf8_lossy(&raw));
        })
    });

    let service = match QuillClient.serve(transport).await {
        Ok(s) => s,
        // 传输层在这一句之前就被丢掉了，`ChildWithCleanup` 的 Drop 会杀掉子进程，
        // 排空任务随之结束。故意不 join 它：握手已经失败了，再等它把 stderr 读完
        // 只是让失败来得更慢。
        Err(e) => return Err(with_stderr(format!("initialize 失败：{e}"), &stderr).await),
    };

    // 协议版本与实现名都是**服务器报什么我们就存什么**。报不出来就说报不出来 ——
    // 拿默认值填上会让「这服务器是谁」看起来像真的。
    let info = service.peer_info().map_or_else(
        || {
            (
                "（服务器没报协议版本）".to_string(),
                "（服务器没报实现名）".to_string(),
            )
        },
        |i| {
            (
                i.protocol_version.to_string(),
                match &i.server_info {
                    Some(s) => format!("{} {}", s.name, s.version),
                    None => "（服务器没报实现名）".to_string(),
                },
            )
        },
    );

    let listed = match service.list_tools(None).await {
        Ok(v) => v,
        Err(e) => {
            let _ = service.cancel().await;
            return Err(with_stderr(format!("tools/list 失败：{e}"), &stderr).await);
        }
    };

    // 分页。只翻**有限**页：服务器可以一直给 next_cursor，翻下去就是个无界循环，
    // 而这条链路在 HTTP 请求里被人等着。
    let mut tools = Vec::new();
    let mut page = listed;
    let mut guard = 0usize;
    loop {
        tools.extend(page.tools.iter().cloned().map(remote_tool));
        guard += 1;
        let Some(cursor) = page.next_cursor.clone() else {
            break;
        };
        if guard >= MAX_PAGES {
            let _ = service.cancel().await;
            let mut msg = format!(
                "tools/list 翻了 {MAX_PAGES} 页还有下一页，已停止。\
                 下一步：这台服务器一次报太多工具，或分页游标有问题。"
            );
            append_stderr(&mut msg, &stderr).await;
            return Err(msg);
        }
        let next = match service
            // `PaginatedRequestParams` 是 `#[non_exhaustive]`，只能走构造器；
            // `meta` 保持 None —— SEP-1319 的协议元数据，这一层不发。
            .list_tools(Some(
                PaginatedRequestParams::default().with_cursor(Some(cursor)),
            ))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let _ = service.cancel().await;
                return Err(with_stderr(format!("tools/list 翻页失败：{e}"), &stderr).await);
            }
        };
        page = next;
    }

    // 收尾：关掉子进程。不做的话每次刷新设备页都会在机器上留一个孤儿进程。
    let _ = service.cancel().await;
    if let Some(h) = drain {
        let _ = h.await;
    }
    Ok(HandshakeTools { tools, info })
}

/// `tools/list` 最多翻几页。`nextCursor` 存在但翻满了还没完，就当它有问题。
const MAX_PAGES: usize = 8;

fn spawn_hint(row: &McpServerRow, e: std::io::Error) -> String {
    let cmd = row.command.clone().unwrap_or_default();
    format!(
        "拉起 `{cmd}` 失败：{e}。\
         下一步：在终端里手动跑一遍 `{}`，确认这个可执行文件存在、且它启动后\
         会往 stdout 写 JSON-RPC（stdio 传输的 stdout 只能是协议内容）。",
        command_line(row)
    )
}

/// 用户可以原样粘进终端的一行命令。
///
/// **参数为空时不留尾随空格**：错误里那个反引号里的内容是要被复制粘贴的，
/// `` `npx ` `` 粘进终端是一个看不出问题的错，而这种「照着做还是错」最难查。
fn command_line(row: &McpServerRow) -> String {
    let mut line = row.command.clone().unwrap_or_default();
    line.push_str(&row.args.join(" "));
    line.trim().to_string()
}

/// 排空后的 stderr 文本。`Default` 出来是空的，所以「还没读到」与「读到了但是空的」
/// 在这里分不开 —— 那种情况下错误里就不附 stderr，比附一段空块诚实。
#[derive(Default)]
struct StderrTail {
    text: tokio::sync::Mutex<String>,
}

impl StderrTail {
    fn set(&self, text: &str) {
        // 只有这个方法持锁，且它由排空任务在句柄被丢弃前调用一次。
        if let Ok(mut g) = self.text.try_lock() {
            g.push_str(text);
        }
    }

    /// 最多等 300 毫秒看一眼。为多打几个字把 HTTP 请求的延迟再拖长不值得，
    /// 所以排空任务还在跑的话就算了 —— 那台服务器本来就已经连不上了。
    async fn snapshot(&self) -> String {
        match tokio::time::timeout(Duration::from_millis(300), async {
            self.text.lock().await.clone()
        })
        .await
        {
            Ok(s) => s,
            Err(_) => String::new(),
        }
    }
}

/// 把服务器自己的 stderr 尾巴附到错误后面。
///
/// 截到尾部而不是全量：有些服务器会往 stderr 里打进度条，界面上要能放下。
async fn with_stderr(mut msg: String, stderr: &Arc<StderrTail>) -> String {
    append_stderr(&mut msg, stderr).await;
    msg
}

async fn append_stderr(msg: &mut String, stderr: &Arc<StderrTail>) {
    let text = stderr.snapshot().await;
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    // 取尾部 600 个字符：从尾巴往前取再翻回来，省得算 UTF-8 边界切出半个字。
    let tail: String = text
        .chars()
        .rev()
        .take(600)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    msg.push_str("\n\n服务器 stderr 末尾：\n");
    msg.push_str(&tail);
}

fn remote_tool(t: McpTool) -> RemoteTool {
    RemoteTool {
        remote_name: t.name.to_string(),
        description: t.description.map(|d| d.to_string()).unwrap_or_default(),
        input_schema: Value::Object((*t.input_schema).clone()),
    }
}

/// 客户端身份。`initialize` 里要报 `clientInfo` —— 不报的话有些服务器会拒绝握手。
#[derive(Debug, Clone, Copy)]
struct QuillClient;

impl ClientHandler for QuillClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("quill", env!("CARGO_PKG_VERSION")),
        )
    }
}

/// 一台服务器在**本轮**的汇总，供响应里的 `note` 如实说话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub probed: usize,
    pub connected: usize,
    pub failed: usize,
}

impl Summary {
    pub fn of(probes: &[Probe]) -> Self {
        Summary {
            probed: probes.iter().filter(|p| p.probed).count(),
            connected: probes.iter().filter(|p| p.connected).count(),
            failed: probes.iter().filter(|p| p.probed && !p.connected).count(),
        }
    }

    /// 顶层 `connected`。**一台都没探测过就是 false** —— 那不是「都连上了」，
    /// 那是「没查过」。界面上靠 `probed` 把这两种情况分开说。
    pub fn all_connected(&self) -> bool {
        self.probed > 0 && self.failed == 0
    }

    /// `note` 的原话。界面**原样显示**这句话，所以每一句都必须是实话，
    /// 而且要说清「没挂进工具表」这件事 —— 连上了不等于模型能调。
    pub fn note(&self, probes: &[Probe]) -> String {
        if self.probed == 0 {
            return "本轮没有发起任何协议握手：没有启用的 stdio 服务器。\
                    配了服务器就能真的 initialize + tools/list。"
                .to_string();
        }
        let mut s = format!(
            "本轮真的对 {} 台 stdio 服务器发起了 initialize + tools/list，{} 台连通、{} 台失败。",
            self.probed, self.connected, self.failed
        );
        let connected_with_tools = probes.iter().filter(|p| p.connected && p.tool_count > 0).count();
        if connected_with_tools > 0 {
            s.push_str(&format!(
                "其中 {connected_with_tools} 台真的报了工具，工具数见每一行。"
            ));
        }
        let capped = probes
            .iter()
            .filter(|p| p.connected && p.tool_count == 0)
            .count();
        if capped > 0 {
            s.push_str(&format!(
                "另有 {capped} 台连上了但工具数是 0：enabled_capabilities 里没有 tools，\
                 或它自己一个工具都没报。"
            ));
        }
        s.push_str("注意：这些工具**还没有挂进对话的工具表**，模型这一轮还调不到它们。");
        s
    }
}

/// 一条 server 状态在响应 JSON 里的形状。**与 `servers` 分开**是有意的：
/// `servers` 会被前端原样回填进编辑表单再 POST 回来，混进只读字段的话
/// `api_extensions::parse_server` 的字段白名单会把它拒掉。
pub fn status_json(p: &Probe) -> Value {
    json!({
        "name": p.name,
        "probed": p.probed,
        "connected": p.connected,
        "tool_count": p.tool_count,
        "error": p.error,
        "protocol_version": p.protocol_version,
        "server_info": p.server_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> McpServerRow {
        McpServerRow {
            name: name.into(),
            transport: "stdio".into(),
            command: Some("true".into()),
            args: vec![],
            env: vec![],
            url: None,
            headers: vec![],
            enabled: true,
            timeout_ms: 30_000,
            description: String::new(),
            cwd: None,
            max_concurrent_calls: None,
            enabled_capabilities: Some(vec![]),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn the_capability_gate_keeps_all_three_states_apart() {
        // 三态一旦被压成布尔，「把全开改成全禁」就看不出区别。
        let mut r = row("fs");
        r.enabled_capabilities = None;
        assert!(!tools_capability_on(&r), "None = 全禁");
        r.enabled_capabilities = Some(vec![]);
        assert!(tools_capability_on(&r), "Some([]) = 全开");
        r.enabled_capabilities = Some(vec!["resources".into()]);
        assert!(!tools_capability_on(&r), "精确列举里没有 tools");
        r.enabled_capabilities = Some(vec!["TOOLS".into()]);
        assert!(tools_capability_on(&r), "能力名大小写不该决定生死");
    }

    #[tokio::test]
    async fn a_disabled_server_is_never_probed() {
        let mut r = row("off");
        r.enabled = false;
        let p = probe(&r).await;
        assert!(!p.probed);
        assert!(!p.connected);
        assert!(p.error.expect("要说明为什么没探测").contains("停用"));
    }

    #[tokio::test]
    async fn an_unimplemented_transport_says_not_implemented_rather_than_connection_failed() {
        // 「还没做」与「连不上」的下一步完全不同，报错的人要能分出来。
        let mut r = row("remote");
        r.transport = "streamable_http".into();
        r.url = Some("https://example.com/mcp".into());
        let p = probe(&r).await;
        assert!(!p.probed);
        let why = p.error.expect("要说明原因");
        assert!(why.contains("还没铺"), "{why}");
        assert!(why.contains("streamable_http"), "{why}");
    }

    #[tokio::test]
    async fn a_missing_command_is_reported_before_any_process_is_spawned() {
        let mut r = row("empty");
        r.command = Some("   ".into());
        let p = probe(&r).await;
        assert!(!p.probed);
        assert!(p.error.expect("要说明原因").contains("command"));
    }

    #[tokio::test]
    async fn a_command_that_does_not_exist_fails_honestly_instead_of_pretending_to_connect() {
        // 这一条是整个模块的底线：起不来的进程必须报失败。
        // 报成 connected=true 的话，界面上会显示一个根本没跑起来的服务器是通的。
        let mut r = row("ghost");
        r.command = Some("quill-no-such-mcp-server-xyzzy".into());
        r.timeout_ms = 5_000;
        let p = probe(&r).await;
        assert!(p.probed, "确实尝试过了");
        assert!(!p.connected, "没起来就不能算连上");
        assert_eq!(p.tool_count, 0);
        let why = p.error.expect("失败必须带原因");
        assert!(why.contains("拉起"), "{why}");
    }

    #[tokio::test]
    async fn a_process_that_never_answers_the_handshake_times_out_with_a_runnable_hint() {
        let mut r = row("mute");
        // `cat` 会一直活着，但从不说 initialize。超时必须是**被观察到的**结果，
        // 不是靠 sleep 猜的。
        r.command = Some("cat".into());
        r.timeout_ms = 1_000;
        let p = probe(&r).await;
        assert!(p.probed);
        assert!(!p.connected);
        let why = p.error.expect("超时也要说原因");
        assert!(why.contains("毫秒"), "{why}");
        assert!(why.contains("cat"), "错误里要带上可手动跑的命令：{why}");
    }

    #[tokio::test]
    async fn a_non_stdio_process_that_exits_at_once_is_reported_as_a_failure() {
        // `true` 立刻退出：管道会关，rmcp 拿到的应该是连接错误而不是成功。
        let mut r = row("quitter");
        r.command = Some("true".into());
        r.timeout_ms = 5_000;
        let p = probe(&r).await;
        assert!(p.probed);
        assert!(!p.connected, "进程秒退不能算连上");
    }

    #[tokio::test]
    async fn probing_nothing_returns_nothing_and_does_not_claim_success() {
        let probes = probe_all(Vec::new()).await;
        assert!(probes.is_empty());
        let s = Summary::of(&probes);
        assert_eq!(s.probed, 0);
        assert!(
            !s.all_connected(),
            "一台都没探测过时报 true，等于告诉界面「都通了」"
        );
        assert!(s.note(&probes).contains("没有发起任何协议握手"));
    }

    #[test]
    fn a_summary_with_one_good_and_one_bad_server_is_not_all_connected() {
        let good = Probe {
            name: "a".into(),
            probed: true,
            connected: true,
            tool_count: 2,
            error: None,
            protocol_version: Some("2025-06-18".into()),
            server_info: Some("srv 1.0".into()),
        };
        let mut bad = good.clone();
        bad.name = "b".into();
        bad.connected = false;
        bad.tool_count = 0;
        bad.error = Some("连不上".into());
        let s = Summary::of(&[good.clone(), bad.clone()]);
        assert_eq!(s.probed, 2);
        assert_eq!(s.connected, 1);
        assert_eq!(s.failed, 1);
        assert!(!s.all_connected());
        let note = s.note(&[good, bad]);
        assert!(note.contains("2 台 stdio 服务器"), "{note}");
        assert!(note.contains("还没有挂进对话的工具表"), "{note}");
    }

    #[test]
    fn a_connected_server_with_capped_capabilities_says_zero_tools_in_the_note() {
        // 连上了但工具被能力开关关掉：tool_count=0 是对的，note 也必须解释。
        let p = Probe {
            name: "a".into(),
            probed: true,
            connected: true,
            tool_count: 0,
            error: None,
            protocol_version: Some("2025-06-18".into()),
            server_info: Some("srv 1.0".into()),
        };
        let s = Summary::of(std::slice::from_ref(&p));
        assert!(s.all_connected());
        assert!(s.note(std::slice::from_ref(&p)).contains("工具数是 0"));
    }

    #[test]
    fn the_probe_budget_never_exceeds_the_ceiling_even_with_a_huge_configured_timeout() {
        // 配置里允许 600 秒。设备页不是批量作业，超出上限就要被截断。
        assert_eq!(probe_budget(1_000).as_millis(), 1_000);
        assert_eq!(probe_budget(600_000).as_millis(), PROBE_BUDGET_CEILING_MS as u128);
        assert_eq!(probe_budget(0).as_millis(), 1_000, "非法值也要给一个能跑的下限");
    }

    #[test]
    fn the_status_shape_keeps_read_only_fields_out_of_the_editable_server_object() {
        // 前端会把 servers 原样回填进编辑表单再 POST 回来；只读字段混进去会被
        // parse_server 的白名单拒掉。
        let v = status_json(&Probe {
            name: "fs".into(),
            probed: true,
            connected: true,
            tool_count: 3,
            error: None,
            protocol_version: Some("2025-06-18".into()),
            server_info: Some("srv 1.0".into()),
        });
        assert_eq!(v["name"], json!("fs"));
        assert_eq!(v["connected"], json!(true));
        assert_eq!(v["tool_count"], json!(3));
        assert!(v["error"].is_null());
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert!(!keys.contains(&&"command".to_string()));
    }
}
