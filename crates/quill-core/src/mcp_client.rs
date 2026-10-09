//! MCP 协议层：**真的**把 stdio 服务器拉起来，真的走一遍 `initialize` + `tools/list`。
//!
//! 分工是硬的：`mcp_repo` 只管存配置（它自己的模块文档里写明了「这里不连服务器」），
//! 本模块是**唯一**发起连接的地方。两者混在一起的后果是「保存配置会因为连不上而失败」，
//! 而连不上往往正是用户需要先把配置登记进去才能排查的东西。
//!
//! **铺了 stdio 与内置（`builtin`）两种。** 内置服务器在内存管道上跑（见
//! [`crate::builtin`]），不起子进程、也不用装东西；`streamable_http` / `sse`
//! 明确报「还没铺」，不假装连过：`probe` 对此给 `probed=false` 加一句白话原因。
//! 报一个「没做」比报一个编出来的 `connected: false` 强 —— 后者会让用户去查
//! 一个根本不存在的网络故障。
//!
//! **命令是发起请求那个用户自己配的。** `mcp_servers` 按 `user_id` 过滤，
//! 拉起谁的进程由谁的配置决定；这里不做任何跨用户兜底，也不接受请求体里
//! 直接塞进来的命令 —— 命令只能来自 `mcp_repo::list` 的读结果。
//!
//! **失败必须说得清。** 握手失败、超时、服务器起不来，三者的下一步完全不同
//! （改配置 / 放宽超时 / 装依赖），所以每条错误都带上一条能照着做的建议。
//!
//! **搬家记录（queue Q014）**：本文件原在 `quill-server/src/mcp_client.rs`，
//! 2026-10-08 原样搬进内核层 —— 它本来就只依赖 `McpServerRow` 与 rmcp、与 HTTP
//! 无关，留在 `quill-server` 里纯属历史原因。行为未改；测试随文件搬走
//! （在 `cargo test -p quill-core` 里跑同一批用例）。
//!
//! 移植出处：`vendor/goose/crates/goose/src/agents/mcp_client.rs`（1687 行，
//! goose v1.53.0）。goose 用同一族 rmcp API（`serve` → `initialize` →
//! `list_tools` → `call_tool`）；quill 铺了 stdio 与 `builtin` 两种传输
//! （后者见 [`crate::builtin`]），其余传输如实报「还没铺」。

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, ContentBlock,
    Implementation, PaginatedRequestParams, Tool as McpTool,
};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::mcp::McpServerRow;

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
    /// 服务器在 `initialize` 里**自报**的 `tools` 能力。
    ///
    /// 与本地 `enabled_capabilities` 是两件事，**都要看**：
    /// 本地开关说「我允许你用 tools」，服务器自报说「我确实有 tools」。
    /// 只看前者的话，一台自报没有 tools 能力的服务器照样能被挂出工具，
    /// 界面上显示「模型调得到」，而模型调过去只会拿到一个方法不存在的错。
    /// 见 ISSUE-014。
    pub server_declares_tools: Option<bool>,
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
            server_declares_tools: None,
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

/// 一次探测的完整结果：**状态**加上**真正会交给模型的工具**。
///
/// 拆成两个字段，是因为它们回答两个不同的问题：「连上了吗」与「模型能调什么」。
/// 只报前者的话，界面上就只剩一个连不上的服务器，和一个说不清的 0。
#[derive(Debug, Clone, PartialEq)]
pub struct Discovery {
    pub probe: Probe,
    /// 按 `enabled_capabilities` 过滤之后真正会挂进工具表的工具。
    /// 连不上时是空的 —— **空列表不等于「它一个工具都没有」**，原因在
    /// `probe.error` 里。这条界线得由类型说清楚，否则调用方会顺手把
    /// 「没连上」渲染成「0 个工具」。
    pub tools: Vec<RemoteTool>,
}

/// 探测一台服务器，**并把工具带回来**。
///
/// 设备页与聊天链路走的是**同一个**函数。两边各握手一次的话，
/// 「界面上说挂了几个」与「模型这一轮真能调几个」迟早对不上，而那种不一致
/// 恰恰是本项目最不能出的错（同一个道理见 `tools::mcp_tool_visibility`）。
///
/// **只握手一次。** 探测会真的把用户配的那个进程拉起来，拉两遍不是「多花一点
/// 时间」，而是让一个慢启动的服务器在每次刷新设备页时多占一份内存，并且
/// 两次结果可能不一样 —— 界面上显示的工具数就会和模型实际拿到的对不上。
///
/// **永远不会 panic**，返回的 `probe` 字段永远完整。
pub async fn discover(row: &McpServerRow) -> Discovery {
    if let Some(why) = not_probed_reason(row) {
        return Discovery {
            probe: Probe::not_probed(row, why),
            tools: Vec::new(),
        };
    }

    let command = row.command.as_deref().unwrap_or("").trim().to_string();
    let budget = probe_budget(row.timeout_ms);
    match tokio::time::timeout(budget, handshake(row)).await {
        Ok(Ok(listed)) => {
            // **两道闸门，都要过。** 本地 `enabled_capabilities` 说「我允许你用
            // tools」，服务器自报说「我确实有 tools」。只看本地那道的话，一台
            // 自报没有 tools 能力的服务器照样能被挂出工具 —— 界面上写「模型
            // 调得到」，而模型调过去只会拿到一个「方法不存在」的错。见 ISSUE-014。
            //
            // 「报 0 而不是报真实条数」：`tools/list` 回了几个是**服务器的行为**，
            // 模型能不能调是**另一件事**，两者在服务器说谎时必须分开报。
            let blocked_by_local = !tools_capability_on(row);
            let blocked_by_server = listed.declares_tools == Some(false);
            let tools = if blocked_by_local || blocked_by_server {
                Vec::new()
            } else {
                listed.tools
            };
            let mut probe = Probe {
                name: row.name.clone(),
                probed: true,
                connected: true,
                tool_count: tools.len(),
                error: None,
                protocol_version: Some(listed.info.0),
                server_info: Some(listed.info.1),
                server_declares_tools: Some(listed.declares_tools.unwrap_or(false)),
            };
            // 「服务器没这个能力」是**正常状态**，不是失败：`connected` 仍然是
            // true。区别由 `error` 这一栏说清，别把它混进连接失败里。
            if blocked_by_local {
                probe.error = Some("本地 enabled_capabilities 里没有 tools，所以一个工具都不给模型。\
                     下一步：在设备页把 tools 加进能力列表，或清空该列表表示全开。".to_string());
            } else if blocked_by_server {
                probe.error = Some("服务器在 initialize 里自报的能力里**没有 tools** —— 它自己说它不提供工具，\
                     所以工具数按 0 报（即使它的 tools/list 回了内容）。\
                     下一步：确认这台服务器是否该提供工具；若它本该提供，那是它 initialize 的问题，\
                     把它的协议版本与实现名连同这段现象反馈给它。".to_string());
            }
            Discovery { probe, tools }
        }
        Ok(Err(why)) => Discovery { probe: Probe::failed(row, why), tools: Vec::new() },
        Err(_) => Discovery {
            probe: Probe::failed(
                row,
                format!(
                    "握手加 tools/list 在 {} 毫秒内没跑完（已按上限截断）。\
                     下一步：先在命令行里手动跑一次 `{command}`，它自己都不回话就别指望协议层能连上。",
                    budget.as_millis()
                ),
            ),
            tools: Vec::new(),
        },
    }
}

/// 这一行**根本没资格发起握手**时的白话原因。`None` = 该去连了。
///
/// 抽出来是因为「不探测」的原因要出现在**两处**（`probed=false` 的那一条，
/// 与聊天链路挂不上工具的那一条），两处各写一遍迟早措辞漂移。
fn not_probed_reason(row: &McpServerRow) -> Option<String> {
    if !row.enabled {
        return Some("已停用（你自己关掉的），本轮没有发起任何连接。".to_string());
    }
    if row.transport == "builtin" {
        let name = row.command.as_deref().unwrap_or("").trim();
        if name.is_empty() {
            return Some(format!(
                "builtin 配置里没写内置服务器的名字。\
                 下一步：把 command 填成其中一个：{}。",
                crate::builtin::names()
            ));
        }
        if !crate::builtin::is_builtin(name) {
            return Some(format!(
                "内置服务器 {name:?} 不认识，本轮没有发起任何连接。\
                 下一步：把 command 改成 {} 之一，或把 transport 改成 stdio 走外部命令。",
                crate::builtin::names()
            ));
        }
        return None;
    }
    if row.transport != "stdio" {
        return Some(format!(
            "传输方式 {} 的协议层还没铺，本轮只铺了 stdio 与 builtin —— 这不是连接失败，是还没做。",
            row.transport
        ));
    }
    if row.command.as_deref().unwrap_or("").trim().is_empty() {
        return Some(
            "stdio 配置里没有 command，没法拉起本地进程。\
             下一步：在这一行填上可执行文件名，例如 `npx`。"
                .to_string(),
        );
    }
    None
}

/// 只要状态、不要工具。设备页与既有调用方用这个。
pub async fn probe(row: &McpServerRow) -> Probe {
    discover(row).await.probe
}

/// 探测一批。**并发**，且是**并发地去连真实的用户进程**：一台起不起来的服务器
/// 慢 10 秒，不该让另外三台跟着排队 —— 界面上「刷新」是一个动作，不该按台数线性变慢。
///
/// 返回值与 `rows` **同序**：调用方按下标对齐，不用按名字再查一遍（名字已归一，
/// 但按名字查会把「哪台对不上」的错误藏起来）。
pub async fn probe_all(rows: Vec<McpServerRow>) -> Vec<Probe> {
    discover_all(rows)
        .await
        .into_iter()
        .map(|d| d.probe)
        .collect()
}

/// `probe_all` 的全量版本：除状态外还带回真正会挂进工具表的工具。
///
/// 同样并发、同样同序。`probe_all` 建在它之上，所以「界面上看到的工具数」与
/// 「模型这一轮真能调的工具」出自**同一次**握手。
pub async fn discover_all(rows: Vec<McpServerRow>) -> Vec<Discovery> {
    let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let mut set = tokio::task::JoinSet::new();
    for (idx, row) in rows.into_iter().enumerate() {
        set.spawn(async move { (idx, discover(&row).await) });
    }
    // **先按输入顺序占好位，再按完成顺序填。** `JoinSet::join_next` 是谁先跑完
    // 先给谁，直接 push 的话返回顺序会随机器负载抖动 —— 而调用方是拿这个下标去
    // 对齐 `servers` 的，顺序一乱，「哪台服务器的工具数」就全串位了。
    let mut slots: Vec<Option<Discovery>> = names.iter().map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((idx, d)) => slots[idx] = Some(d),
            // 探测任务自己不会 panic（`discover` 内部没有 unwrap）。真出了 join 错误，
            // 也不能让整个接口 500 —— 那是「一个用户进程有问题」变成「整页 MCP 打不开」。
            // 槽位留着 None，下面按名字填成一条「没探测成」的记录。
            Err(e) => eprintln!("[mcp] 探测任务没能跑完：{e}"),
        }
    }
    slots
        .into_iter()
        .zip(names)
        .map(|(slot, name)| {
            slot.unwrap_or_else(|| Discovery {
                probe: Probe {
                    name,
                    probed: false,
                    connected: false,
                    tool_count: 0,
                    error: Some("探测任务没能跑完（见服务端日志）".to_string()),
                    protocol_version: None,
                    server_info: None,
                    server_declares_tools: None,
                },
                tools: Vec::new(),
            })
        })
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
    /// 服务器在 `initialize` 里自报有没有 `tools` 能力。`None` = 它没报。
    ///
    /// `None` 与 `Some(false)` 分开，是因为「没报能力」和「明确说没有工具」
    /// 面对的是两种不同的服务器：前者有一批老实现根本不写 `capabilities`
    /// 却照样能调 `tools/list`，照着 `Some(false)` 处理会把它们全打成 0。
    declares_tools: Option<bool>,
}

async fn handshake(row: &McpServerRow) -> Result<HandshakeTools, String> {
    let Connected {
        service,
        stderr,
        drain,
    } = connect(row).await?;

    // 协议版本与实现名都是**服务器报什么我们就存什么**。报不出来就说报不出来 ——
    // 拿默认值填上会让「这服务器是谁」看起来像真的。
    //
    // 顺手把服务器**自报的能力**也读出来（ISSUE-014）：本地开关说「我允许你用
    // tools」，服务器自报说「我确实有 tools」，两者都要看。只看本地那道，
    // 一台自报没有 tools 的服务器会被挂出工具，界面上写「模型调得到」。
    let (info, declares_tools) = service.peer_info().map_or_else(
        || {
            (
                (
                    "（服务器没报协议版本）".to_string(),
                    "（服务器没报实现名）".to_string(),
                ),
                // 没报 peer_info 就无从知道它报没报能力 —— 记成「没报」，
                // 而不是替它猜一个 false。
                None,
            )
        },
        |i| {
            (
                (
                    i.protocol_version.to_string(),
                    match &i.server_info {
                        Some(s) => format!("{} {}", s.name, s.version),
                        None => "（服务器没报实现名）".to_string(),
                    },
                ),
                Some(i.capabilities.tools.is_some()),
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
    Ok(HandshakeTools {
        tools,
        info,
        declares_tools,
    })
}

/// `tools/list` 最多翻几页。`nextCursor` 存在但翻满了还没完，就当它有问题。
const MAX_PAGES: usize = 8;

/// 拉起一个 stdio 子进程，并**同时**开始排空它的 stderr。
///
/// `spawn_stdio` 的返回：`(传输层, stderr 累积器, 排空任务句柄)`。
/// 排空句柄是 `Option`：传输层在没接 stderr 时不会给我们一个。
type SpawnedStdio = (TokioChildProcess, Arc<StderrTail>, Option<JoinHandle<()>>);

/// 一台服务器**连上之后**的东西：客户端服务 + 两个诊断件。
struct Connected {
    service: rmcp::service::RunningService<RoleClient, QuillClient>,
    stderr: Arc<StderrTail>,
    drain: Option<JoinHandle<()>>,
}

/// 连上这台服务器。**两条传输在这里合流**：`stdio` 拉子进程、`builtin` 在内存管道上跑
/// （见 [`crate::builtin`]）。
///
/// 合流的理由与下面 `spawn_stdio` 头注里那条一样：探测与 `tools/call` 必须走**同一套**
/// 连接形状。只不过现在多了一种传输，如果两条各自实现一遍，「探测时连的」与
/// 「调用工具时连的」就不只是 cwd/env 会漂，而是**连的根本不是同一台服务器**。
///
/// 收尾（`service.cancel()` 与等排空任务）仍留在调用方：探测与调用对「什么时候该收尾」
/// 不一样（探测早收，调用要等结果渲染完）。
async fn connect(row: &McpServerRow) -> Result<Connected, String> {
    if row.transport == "builtin" {
        let name = row.command.as_deref().unwrap_or("").trim().to_string();
        let (read, write) = crate::builtin::spawn(&name).ok_or_else(|| {
            format!(
                "内置服务器 {name:?} 不认识，没有发起连接。\
                 下一步：把 command 改成 {} 之一。",
                crate::builtin::names()
            )
        })?;
        let service = QuillClient.serve((read, write)).await.map_err(|e| {
            format!(
                "内置服务器 {name} 的 initialize 失败：{e}。\
                 下一步：它是随 quill 自带的（不起子进程、不用装依赖），连不上就是 quill 自己的\
                 问题 —— 请带上这一句报一个 issue。"
            )
        })?;
        return Ok(Connected {
            service,
            stderr: Arc::new(StderrTail::default()),
            drain: None,
        });
    }

    let (transport, stderr, drain) = spawn_stdio(row)?;
    let service = match QuillClient.serve(transport).await {
        Ok(s) => s,
        // 传输层在这一句之前就被丢掉了，`ChildWithCleanup` 的 Drop 会杀掉子进程，
        // 排空任务随之结束。故意不 join 它：握手已经失败了，再等它把 stderr 读完
        // 只是让失败来得更慢。
        // 面向用户的错误**一律带「下一步」** —— 这一句会原样进界面。
        Err(e) => {
            return Err(with_stderr(
                format!(
                    "initialize 失败：{e}。\
                     下一步：在终端里手动跑一次 `{}`，看它启动后会不会往 stdout 写 JSON-RPC\
                     （stdio 传输的 stdout 只能是协议内容）；下面附的 stderr 末尾通常就是原因。",
                    command_line(row)
                ),
                &stderr,
            )
            .await)
        }
    };
    Ok(Connected {
        service,
        stderr,
        drain,
    })
}

/// 探测与 `tools/call` 走的是**同一个**拉起函数。分成两份的话，「探测时用的
/// cwd/env」与「真正调用工具时用的 cwd/env」就会漂，而那种漂移只在某一个
/// 特定工具上发作，极难查。
fn spawn_stdio(row: &McpServerRow) -> Result<SpawnedStdio, String> {
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
    Ok((transport, stderr, drain))
}

// --------------------------------------------------------------- tools/call

/// 调一次 MCP 工具的预算上限。配置里的 `timeout_ms` 最大 600 秒，但那是**用户**
/// 给的预算；一次对话请求在工具上吊 10 分钟，比调不到工具更糟。上限取 60 秒，
/// 超时的话错误里会写清「你配的是多少、按多少截断的」。
pub const CALL_BUDGET_CEILING_MS: i64 = 60_000;

/// 闸门等待的额外余量。超时判定在 `call_tool` 内部已经做完了，这里多给一点
/// 是为了让「真的超时了」和「刚好卡在闸门上」这两种错误能被区分开。
const GATE_SLACK: Duration = Duration::from_millis(500);

/// `max_concurrent_calls` 留空时取几。stdio 服务器靠一对管道应答，并发拉高只会
/// 让它们互相拖累；而在这台机器上「不限制」实际等于「一个卡死的工具把对话卡死」。
pub const DEFAULT_MAX_CONCURRENT_CALLS: usize = 4;

/// 每台服务器的并发闸门。键是 `(用户, 服务器名)`。
///
/// 键里带用户是必须的：两个用户各自配了一台同名服务器时不该共用一个闸门 ——
/// 那等于让 B 的调用数去限制 A，而界面上谁也看不出来。
static GATES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>,
> = std::sync::OnceLock::new();

/// 拿到（或第一次建）某台服务器的闸门。
fn gate(user_key: &str, row: &McpServerRow) -> Arc<tokio::sync::Semaphore> {
    let limit = match row.max_concurrent_calls {
        Some(n) if n >= 1 => n as usize,
        Some(_) => 1,
        None => DEFAULT_MAX_CONCURRENT_CALLS,
    };
    let key = format!("{user_key}\u{1}{}", row.name);
    let mut guard = GATES
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(limit))),
    )
}

/// 真的调一次 MCP 工具：`initialize` → `tools/call` → 收正文。
///
/// **每次调用都重新拉起一个子进程。** 不复用探测那次的进程是有意的：探测在返回前
/// 就把子进程收了（`service.cancel()`），而对话可能几分钟后才调这个工具。跨请求
/// 养着一个子进程意味着要管它的生命周期、崩溃、以及「用户改完配置之后那个进程还
/// 按旧配置跑着」。stdio 服务器本来就是一次性会话的形态，一次调用一次拉起更贴近
/// 它的本意，代价是每次调用多付一次进程启动的时间。
pub async fn call_tool(
    row: &McpServerRow,
    user_key: &str,
    remote_name: &str,
    args: &Value,
) -> Result<String, String> {
    if let Some(why) = not_probed_reason(row) {
        return Err(format!(
            "这台服务器这一轮没有发起过连接，它的工具调不到：{why}\
             下一步：到设备页确认这一行是启用状态、传输方式是 stdio、并且填了 command。"
        ));
    }
    if !tools_capability_on(row) {
        return Err(format!(
            "服务器 {} 的 enabled_capabilities 里没有 tools，模型看不到它的工具。\
             下一步：在设备页把 tools 加进能力列表，或清空该列表表示全开。",
            row.name
        ));
    }
    let args = match args {
        Value::Object(o) => o.clone(),
        other => {
            return Err(format!(
                "工具参数必须是一个 JSON 对象，收到的是 {other}。\
                 下一步：按该工具在 tools/list 里报的 inputSchema 传参。"
            ))
        }
    };

    let semaphore = gate(user_key, row);
    let permits = semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("并发闸门关不上了：{e}"))?;

    let limit = row.timeout_ms.max(1_000);
    let budget = Duration::from_millis(limit.min(CALL_BUDGET_CEILING_MS) as u64);
    let out = tokio::time::timeout(budget, invoke(row, remote_name, args)).await;
    // **先放闸门再解释结果**：调用已经结束了，闸门没理由还占着 ——
    // 不放的话，一个超时的工具会把同服务器的其它调用一起堵到超时。
    drop(permits);
    match out {
        Ok(r) => r,
        Err(_) => Err(format!(
            "调 `{}` 在 {} 毫秒内没回（按上限截断{}）。\
             下一步：先在命令行里手动跑一次 `{}` 验证它自己能不能回话；\
             如果确实要更久，把这一行的超时调大（上限 {} 秒）。",
            remote_name,
            budget.as_millis(),
            if limit > CALL_BUDGET_CEILING_MS {
                format!("，你配的是 {} 毫秒", limit)
            } else {
                String::new()
            },
            command_line(row),
            CALL_BUDGET_CEILING_MS / 1000,
        )),
    }
}

/// `tools/call` 的一次完整往返。**只在这里**收尾进程，握手失败也一样。
async fn invoke(
    row: &McpServerRow,
    remote_name: &str,
    args: Map<String, Value>,
) -> Result<String, String> {
    let Connected {
        service,
        stderr,
        drain,
    } = connect(row).await?;

    // 调用这一侧**也要**过服务器自报那道闸门（ISSUE-014）。挂载侧已经按 0
    // 处理了，但工具是模型按名字直接调进来的 —— 一台自报没有 tools 的服务器
    // 就算侥幸挂上了，调过去也只会拿到一个「方法不存在」。这里在**同一次**
    // 握手里判，不额外拉进程。
    if service
        .peer_info()
        .is_some_and(|i| i.capabilities.tools.is_none())
    {
        let _ = service.cancel().await;
        return Err(with_stderr(
            format!(
                "服务器 {} 在 initialize 里自报的能力里没有 tools，它自己说它不提供工具。\
                 下一步：确认这台服务器是否该提供工具；若它本该提供，那是它 initialize 的问题。",
                row.name
            ),
            &stderr,
        )
        .await);
    }

    let mut params = CallToolRequestParams::new(remote_name.to_string());
    params.arguments = Some(args);
    // 用 `call_tool_once` 而不是 `call_tool`：前者会把「服务器要客户端补输入」
    // 与「服务器把这次调用变成了一个 task」这两种结果**分别**报出来。
    // 压成 `call_tool` 的话两者都只剩一句 UnexpectedResponse ——
    // 而「下一步」在这两种情况下完全不同。
    let response = match service.call_tool_once(params).await {
        Ok(v) => v,
        Err(e) => {
            let _ = service.cancel().await;
            // 「Transport closed」这类错误**不区分**是哪一步断的：服务器崩了、
            // 管道断了、它自己关了 stdio，回来的都是这一句。所以这里不替它猜原因，
            // 只把「怎么查」说清楚，并把服务器自己的 stderr 附上 ——
            // 那才是真正写着原因的地方。
            return Err(with_stderr(
                format!(
                    "tools/call 失败：{e}。\
                     下一步：先在命令行里手动跑一次 `{}` 再发一次同样的调用，\
                     看它的输出与 stderr；若它自己都会崩，quill 这边修不了，得先修服务器。",
                    command_line(row)
                ),
                &stderr,
            )
            .await);
        }
    };

    let rendered = render_call_result(remote_name, &response);
    // 收尾：关掉子进程。不做的话每次调用都会在机器上留一个孤儿进程。
    let _ = service.cancel().await;
    if let Some(h) = drain {
        let _ = h.await;
    }
    match rendered {
        Ok(text) => Ok(text),
        Err(why) => Err(with_stderr(why, &stderr).await),
    }
}

/// 把 `tools/call` 的响应整理成**给模型看的文本**。
///
/// **只回文本。** 图片/音频/内嵌资源这一层不处理，如实说一句「这次回了非文本内容」——
/// 把它们悄悄丢掉的话，模型会以为工具什么都没返回，于是自己编一个结论。
/// 结构化结果（`structuredContent`）在没有任何文本时顶上，顺序不反过来：
/// 一个既给了文本又给了结构体的工具，文本才是它想让人读的那份。
fn render_call_result(remote_name: &str, response: &CallToolResponse) -> Result<String, String> {
    let result = match response {
        CallToolResponse::Complete(r) => r,
        CallToolResponse::InputRequired(_) => {
            return Err(format!(
                "服务器 `{}` 要求客户端补充输入才能完成这次调用，这一层还不支持\
                 （MRTR 多轮输入）。\
                 下一步：换一条不依赖交互输入的提示词，或换一台不要求补输入的服务器。",
                remote_name
            ))
        }
        CallToolResponse::Task(_) => {
            return Err(format!(
                "服务器 `{}` 把这次调用变成了一个后台任务（tasks 扩展），这一层还不支持轮询它。\
                 下一步：换一台同步返回结果的服务器，或直接调用它对应的命令行工具。",
                remote_name
            ))
        }
        // `#[non_exhaustive]`：以后 rmcp 加一种结果类型，这里会先撞上。
        // 撞上时报「不认识」而不是猜一种 —— 猜错的话就是把一次调用结果
        // 编成另一种形状再喂给模型。
        other => {
            return Err(format!(
                "服务器 `{}` 回了一种这一层还不认识的结果类型（{other:?}）。\
                 下一步：升级 quill，或换一台只回文本的服务器。",
                remote_name
            ))
        }
    };

    let mut parts: Vec<String> = Vec::new();
    let mut non_text = 0usize;
    for block in &result.content {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            other => {
                non_text += 1;
                let kind = match other {
                    ContentBlock::Image(_) => "图片",
                    ContentBlock::Audio(_) => "音频",
                    ContentBlock::Resource(_) => "内嵌资源",
                    ContentBlock::ResourceLink(_) => "资源链接",
                    // 同上：非穷尽。没见过的东西就说「没见过的内容」，
                    // 不硬套一个已知的名字 —— 套错了模型会照着错的类型理解。
                    _ => "这一层还不认识的内容",
                };
                parts.push(format!("（服务器回了一个{kind}，这一层只把文本喂给模型）"));
            }
        }
    }
    if parts.is_empty() {
        if let Some(sc) = &result.structured_content {
            parts.push(
                serde_json::to_string_pretty(sc)
                    .unwrap_or_else(|e| format!("（结构化结果序列化失败：{e}）")),
            );
        }
    }
    if parts.is_empty() {
        parts.push("（服务器执行成功，但没有返回任何内容）".to_string());
    }
    if non_text > 0 {
        parts.push(format!(
            "（本次共 {non_text} 个非文本内容块没有原样传下去，模型只看到了上面这些文字。）"
        ));
    }

    let text = parts.join("\n");
    // `isError` 走 **Err**：错误文本会回灌给模型，让它自己纠正；
    // 走 Ok 的话模型会把失败当成功，接着编一个基于失败的结论。
    if result.is_error == Some(true) {
        return Err(format!("服务器报这次调用失败：{text}"));
    }
    Ok(text)
}

/// `call_tool` 的**同步**入口 —— 给 `ToolHandler` 用。
///
/// **为什么另起一条线程，而不是 `Handle::current().block_on`。**
/// `ToolHandler` 的签名是 `Fn(&Value) -> Result<String, String>`（同步），
/// 而 `tools/call` 是 async。桥只有三条路：
///
/// 1. `Handle::block_on` —— 在 async 上下文里**直接 panic**
///    （`tokio` 明令不许嵌套驱动），而且 `#[tokio::test]` 默认是
///    current-thread 运行时，集成测试会当场炸掉。
/// 2. `block_in_place` + `block_on` —— 在 current-thread 运行时上同样 panic，
///    测试照样炸；等于把「能不能跑」押在测试怎么配置上。
/// 3. **自己开一个线程与一个 current-thread 运行时**（这里选的）。它跟外面
///    的运行时是什么 flavor 无关，多付的代价是一次线程创建（微秒级），
///    换来的是「在任何地方都能调」。
///
/// 代价要说清楚：调用方的那个 worker 线程会**阻塞**在这里等结果
/// （`ToolHandler` 是同步的，这没法绕开）。`MAX_TOOL_ROUNDS` 与
/// `CALL_BUDGET_CEILING_MS` 一起给这段时间封了顶。
pub fn call_tool_blocking(
    row: &McpServerRow,
    user_key: &str,
    remote_name: &str,
    args: &Value,
) -> Result<String, String> {
    let row = row.clone();
    let user_key = user_key.to_string();
    let name = remote_name.to_string();
    let args = args.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    // 错误文案里要用的两样东西，**在 `row` 被 move 进闭包之前**取好。
    let line = command_line(&row);
    let wait = Duration::from_millis(row.timeout_ms.clamp(1_000, CALL_BUDGET_CEILING_MS) as u64)
        + GATE_SLACK;

    let worker = std::thread::Builder::new()
        .name("quill-mcp-tool".to_string())
        .spawn(move || {
            let out = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(call_tool(&row, &user_key, &name, &args)),
                Err(e) => Err(format!(
                    "起不起执行 MCP 工具的环境：{e}。\
                     下一步：重启服务再试；若反复出现，检查这台机器的线程/内存是否已耗尽。"
                )),
            };
            let _ = tx.send(out);
        });

    if let Err(e) = worker {
        return Err(format!(
            "起不起执行 MCP 工具的线程：{e}。\
             下一步：重启服务再试；若反复出现，检查这台机器的线程数是否已耗尽。"
        ));
    }
    // 故意**不 join** 那个线程：它可能正卡在收尾上，等它等于把超时又拖长一遍。
    // 句柄在这里被丢弃 = 分离，线程自己会结束。
    match rx.recv_timeout(wait) {
        Ok(out) => out,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "执行 `{remote_name}` 的线程在 {} 毫秒内没交回结果。\
             下一步：先在命令行里手动跑一次 `{line}` 看它能不能回话。",
            wait.as_millis(),
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(format!(
            "执行 `{remote_name}` 的线程没能把结果送回来（它可能在建立连接时就崩了）。\
             下一步：看服务端日志里这一段，或先在命令行里手动跑一次 `{line}`。"
        )),
    }
}

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
        tokio::time::timeout(Duration::from_millis(300), async {
            self.text.lock().await.clone()
        })
        .await
        .unwrap_or_default()
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

    /// `note` 的原话。界面**原样显示**这句话，所以每一句都必须是实话。
    ///
    /// 这里**只讲协议层看到的东西**：「挂没挂进工具表」不在这一句里 ——
    /// 那是 `tools::mcp_tool_visibility` 的判断，由 `api_extensions::mcp_body`
    /// 拿到挂载结果之后**追加**在后面。分两处讲，是因为「连上了」与
    /// 「模型调得到」是两个问题，合成一句就会出现「3 个工具可用」而模型
    /// 一个都调不到的情况。
    pub fn note(&self, probes: &[Probe]) -> String {
        if self.probed == 0 {
            return "本轮没有发起任何协议握手：没有启用的 MCP 服务器。\
                    配了服务器就能真的 initialize + tools/list。"
                .to_string();
        }
        let mut s = format!(
            "本轮真的对 {} 台 MCP 服务器发起了 initialize + tools/list，{} 台连通、{} 台失败。",
            self.probed, self.connected, self.failed
        );
        let connected_with_tools = probes
            .iter()
            .filter(|p| p.connected && p.tool_count > 0)
            .count();
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
        // 「服务器自报没有 tools 能力」与上面两种都不是一回事，单独说。
        let self_declared_off = probes
            .iter()
            .filter(|p| p.connected && p.server_declares_tools == Some(false))
            .count();
        if self_declared_off > 0 {
            s.push_str(&format!(
                "其中 {self_declared_off} 台在 initialize 里自报的能力里**没有 tools** —— \
                 按 0 个工具报，不管它的 tools/list 回没回内容（ISSUE-014）。"
            ));
        }
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
        "server_declares_tools": p.server_declares_tools,
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
            server_declares_tools: Some(true),
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
        assert!(note.contains("2 台 MCP 服务器"), "{note}");
        // 措辞里不许再写死「stdio」：内置（builtin）行也走这条 note，
        // 写死的话界面上会对着（内存管道里的）内置服务器说「stdio 服务器」。
        assert!(!note.contains("stdio"), "{note}");
        // `Summary::note` **只**讲协议层看到的东西。「挂没挂进工具表」是
        // `tools::mcp_tool_visibility` 的判断，由 `api_extensions::mcp_body`
        // 拿到挂载结果之后追加。这里断言它**不**碰那件事 ——
        // 早先这里写死「还没有挂进对话的工具表」，`with_mcp_tools` 接上之后
        // 那句话就成了假话，而留着它等于逼着后来的人把真功能改回去。
        assert!(
            !note.contains("挂进对话"),
            "挂载状态由 mcp_body 追加，Summary::note 不该碰这件事：{note}"
        );
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
            server_declares_tools: Some(true),
        };
        let s = Summary::of(std::slice::from_ref(&p));
        assert!(s.all_connected());
        assert!(s.note(std::slice::from_ref(&p)).contains("工具数是 0"));
    }

    #[test]
    fn the_probe_budget_never_exceeds_the_ceiling_even_with_a_huge_configured_timeout() {
        // 配置里允许 600 秒。设备页不是批量作业，超出上限就要被截断。
        assert_eq!(probe_budget(1_000).as_millis(), 1_000);
        assert_eq!(
            probe_budget(600_000).as_millis(),
            PROBE_BUDGET_CEILING_MS as u128
        );
        assert_eq!(
            probe_budget(0).as_millis(),
            1_000,
            "非法值也要给一个能跑的下限"
        );
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
            server_declares_tools: Some(true),
        });
        assert_eq!(v["name"], json!("fs"));
        assert_eq!(v["connected"], json!(true));
        assert_eq!(v["tool_count"], json!(3));
        assert!(v["error"].is_null());
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert!(!keys.contains(&&"command".to_string()));
    }

    // ---------------------------------------------------------------- builtin

    /// 一行 `transport = 'builtin'` 的配置。`command` 里写**名字**，不是路径。
    fn builtin_row(name: &str, server: Option<&str>) -> McpServerRow {
        let mut r = row(name);
        r.transport = "builtin".into();
        r.command = server.map(|s| s.to_string());
        r
    }

    #[tokio::test]
    async fn a_builtin_server_is_probed_over_an_in_memory_pipe() {
        // 内置服务器**不拉子进程**：两对 `tokio::io::duplex` 交叉接上就成。
        // 这条钉住三件：真的完成了握手、四个工具都在、服务器自报工具能力。
        let r = builtin_row("mem", Some("memory"));
        let d = discover(&r).await;
        assert!(d.probe.probed, "内置服务器该真的握手：{:?}", d.probe);
        assert!(d.probe.connected, "{:?}", d.probe);
        assert_eq!(d.probe.tool_count, 4, "{:?}", d.probe);
        let names: Vec<&str> = d.tools.iter().map(|t| t.remote_name.as_str()).collect();
        for want in [
            "remember_memory",
            "retrieve_memories",
            "remove_memory_category",
            "remove_specific_memory",
        ] {
            assert!(names.contains(&want), "缺工具 {want}，实际 {names:?}");
        }
        assert!(
            d.probe
                .server_info
                .as_deref()
                .unwrap_or("")
                .contains("quill-memory"),
            "服务器要自报名字：{:?}",
            d.probe
        );
        assert_eq!(
            d.probe.server_declares_tools,
            Some(true),
            "内置服务器的能力是**实测**的，不是本地开关推的：{:?}",
            d.probe
        );
    }

    #[tokio::test]
    async fn an_unknown_builtin_name_is_refused_before_any_connection() {
        let r = builtin_row("mem", Some("nope"));
        let p = probe(&r).await;
        assert!(!p.probed, "名字不认识就不该发起连接：{:?}", p);
        let why = p.error.expect("要说明为什么没探测");
        assert!(why.contains("nope"), "要点名那个不认识的名字：{why}");
        assert!(why.contains("memory"), "要列出可用的内置名：{why}");
    }

    #[tokio::test]
    async fn a_builtin_row_without_a_name_is_reported() {
        let r = builtin_row("mem", None);
        let p = probe(&r).await;
        assert!(!p.probed);
        assert!(
            p.error.expect("要说明原因").contains("command"),
            "要告诉用户名字写在 command 里"
        );
    }

    #[tokio::test]
    async fn a_tool_call_round_trips_over_the_in_memory_pipe() {
        // 过**真协议**调一次工具。挑 `retrieve_memories`（只读）：它不落盘，
        // 所以这条用例不会在工作目录里留下 `.quill/memory`。
        let r = builtin_row("mem", Some("memory"));
        let out = call_tool(
            &r,
            "u1",
            "retrieve_memories",
            &serde_json::json!({"category": "*", "is_global": false}),
        )
        .await
        .expect("内置服务器该真的能调");
        assert!(
            out.contains("Retrieved memories"),
            "回的是记忆服务器的话术，实际 {out:?}"
        );
    }
}
