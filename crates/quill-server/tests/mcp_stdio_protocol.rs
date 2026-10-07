//! MCP 协议层的端到端测试：**真的起一个子进程**，真的握手，真的 `tools/list`。
//!
//! 对端是 `src/bin/mcp_stub.rs` —— 一个手写的 stdio MCP 服务器，只按规范字面说话。
//! 用 rmcp 自己起个 server 当对端也能过，但那样的测试只能证明「rmcp 和 rmcp 对得上」：
//! 协议层写错了，两边会一起错。所以对端刻意不共用 rmcp 的任何类型。
//!
//! 这里断言的每一条都是**观察到的行为**，不是「应该会」：
//! 工具数是数出来的，协议版本是握手结果里读出来的，失败是子进程真的没起来。

use quill_server::mcp_client;
use quill_server::mcp_repo::McpServerRow;

fn stub() -> String {
    std::env::var("CARGO_BIN_EXE_quill-mcp-stub")
        .unwrap_or_else(|e| panic!("拿不到 stub 的路径，集成测试没法起对端：{e}"))
}

fn row(name: &str, command: &str, args: &[&str]) -> McpServerRow {
    McpServerRow {
        name: name.into(),
        transport: "stdio".into(),
        command: Some(command.into()),
        args: args.iter().map(|s| (*s).to_string()).collect(),
        env: vec![],
        url: None,
        headers: vec![],
        enabled: true,
        timeout_ms: 20_000,
        description: String::new(),
        cwd: None,
        max_concurrent_calls: None,
        // 默认「能力全开」：这一层测的是协议，不是能力开关。
        enabled_capabilities: Some(vec![]),
        created_at: 0,
        updated_at: 0,
    }
}

fn as_stub(name: &str, args: &[&str]) -> McpServerRow {
    row(name, &stub(), args)
}

/// 端到端的主线：握手 → tools/list（含分页）→ 工具数与协议版本都对得上。
#[tokio::test]
async fn a_real_stdio_server_completes_the_handshake_and_reports_its_tools() {
    let p = mcp_client::probe(&as_stub("notes", &[])).await;
    assert!(p.probed, "确实发起了握手：{p:?}");
    assert!(p.connected, "握手应当成功：{p:?}");
    assert_eq!(p.error, None, "连上了就不该带错误：{p:?}");
    assert_eq!(
        p.protocol_version.as_deref(),
        Some("2025-06-18"),
        "协议版本要如实带回服务端报的那个：{p:?}"
    );
    assert_eq!(
        p.server_info.as_deref(),
        Some("quill-test-stub 1.0.0"),
        "实现名与版本要如实带回：{p:?}"
    );
    // 3 = 首页 2 个 + 翻游标之后的 1 个。不翻第二页就只能是 2，
    // 而 2 看上去也「像个整数」，页面上分辨不出来。
    assert_eq!(p.tool_count, 3, "分页必须真的翻完：{p:?}");
}

#[tokio::test]
async fn the_capability_gate_zeroes_the_tool_count_without_faking_a_connection() {
    // 连上了、但 `enabled_capabilities` 没有 tools：工具数必须是 0，
    // 而 `connected` 仍然是 true —— 这两件事分开报，不许混成一个。
    let mut r = as_stub("capped", &[]);
    r.enabled_capabilities = Some(vec!["resources".into()]);
    let p = mcp_client::probe(&r).await;
    assert!(p.connected, "握手本身是成功的：{p:?}");
    assert_eq!(p.tool_count, 0, "模型看不到的工具不该被算进去：{p:?}");
}

#[tokio::test]
async fn a_server_that_reports_no_tools_is_connected_with_zero() {
    let p = mcp_client::probe(&as_stub("empty", &["--no-tools"])).await;
    assert!(p.connected, "握手成功、tools/list 成功，只是没工具：{p:?}");
    assert_eq!(p.tool_count, 0);
}

#[tokio::test]
async fn a_rejected_initialize_is_reported_as_a_failure_with_the_servers_own_words() {
    // 服务端回 JSON-RPC 错误：客户端要把它的话带给用户，不能吞成「未知错误」。
    let p = mcp_client::probe(&as_stub("angry", &["--bad-initialize"])).await;
    assert!(p.probed, "确实尝试过了：{p:?}");
    assert!(!p.connected, "被拒绝的握手不是握手成功：{p:?}");
    assert_eq!(p.tool_count, 0);
    let why = p.error.expect("失败必须带原因");
    assert!(why.contains("initialize"), "要说清卡在哪一步：{why}");
}

#[tokio::test]
async fn a_server_that_dies_after_the_handshake_is_not_reported_as_connected() {
    let p = mcp_client::probe(&as_stub("quitter", &["--die-on-tools-list"])).await;
    assert!(p.probed, "握手那一步过了，所以算探测过：{p:?}");
    assert!(!p.connected, "tools/list 没拿到结果就不算连上：{p:?}");
    assert_eq!(p.tool_count, 0);
    let why = p.error.expect("失败必须带原因");
    assert!(why.contains("tools/list"), "{why}");
}

#[tokio::test]
async fn a_silent_server_times_out_instead_of_hanging_the_page_forever() {
    // `--stutter` 永远不回话。这条专门钉住「设备页不会被一台坏服务器挂死」。
    let mut r = as_stub("mute", &["--stutter"]);
    r.timeout_ms = 1_000;
    let started = std::time::Instant::now();
    let p = mcp_client::probe(&r).await;
    let spent = started.elapsed();
    assert!(p.probed && !p.connected, "{p:?}");
    assert!(
        p.error.as_deref().expect("要有原因").contains("毫秒"),
        "{p:?}"
    );
    assert!(
        spent < std::time::Duration::from_secs(15),
        "1 秒的超时不该跑成 {spent:?} —— 上限没生效的话整页会跟着卡住"
    );
}

#[tokio::test]
async fn the_stderr_of_a_broken_server_reaches_the_error_message() {
    // 用户能照着 stderr 里的提示去修。丢掉它的话，界面上只剩「连不上」。
    let p = mcp_client::probe(&as_stub("noisy", &["--die-on-tools-list"])).await;
    let why = p.error.expect("失败必须带原因");
    assert!(
        why.contains("stub: 按要求在 tools/list 之前退出"),
        "服务器的 stderr 应当被带进错误里：{why}"
    );
}

#[tokio::test]
async fn the_env_and_cwd_from_the_row_really_reach_the_child_process() {
    // cwd 配错时的报错里必须带着那个路径 —— 不然用户不知道自己 cwd 写错了。
    let mut r = as_stub("nowhere", &[]);
    r.cwd = Some("/quill-no-such-dir-xyzzy".into());
    let p = mcp_client::probe(&r).await;
    assert!(p.probed);
    assert!(!p.connected, "cwd 不存在就该连不上：{p:?}");
    let why = p.error.expect("失败必须带原因");
    assert!(why.contains("拉起"), "{why}");
    assert!(why.contains("手动跑一遍"), "错误要带得动手的下一步：{why}");
}

#[tokio::test]
async fn probing_a_batch_keeps_the_row_order_and_probes_every_row() {
    // 顺序必须与 `rows` 一致：调用方靠下标把 `servers` 与 `status` 对齐。
    let rows = vec![
        as_stub("first", &[]),
        row("second", "quill-no-such-mcp-server-xyzzy", &[]),
        as_stub("third", &["--one-page"]),
    ];
    let probes = mcp_client::probe_all(rows).await;
    assert_eq!(probes.len(), 3, "一台都不能少：{probes:?}");
    assert_eq!(
        probes.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        vec!["first", "second", "third"],
        "返回顺序必须与输入一致"
    );
    assert!(probes[0].connected, "{:?}", probes[0]);
    assert!(!probes[1].connected, "{:?}", probes[1]);
    assert!(probes[2].connected, "{:?}", probes[2]);
    assert_eq!(probes[2].tool_count, 3, "不分页时三个工具一次给全");

    let s = mcp_client::Summary::of(&probes);
    assert_eq!((s.probed, s.connected, s.failed), (3, 2, 1));
    assert!(!s.all_connected());
}

#[tokio::test]
async fn a_disabled_row_stays_out_of_the_summary_counts() {
    // 停用的服务器不该让「本轮探测了 N 台」虚高。
    let mut off = as_stub("off", &[]);
    off.enabled = false;
    let probes = mcp_client::probe_all(vec![as_stub("on", &[]), off]).await;
    let s = mcp_client::Summary::of(&probes);
    assert_eq!(s.probed, 1, "只数真的探测过的：{probes:?}");
    assert_eq!(s.connected, 1);
    assert_eq!(s.failed, 0);
    assert!(s.all_connected());
}

/// 进程清理：探测一次不该在机器上留下一个还活着的子进程。
///
/// **两个前提缺一不可**，否则这条测试是假的：
///
/// 1. `--hold-open` —— 对端在 stdin 关掉之后**不自己退出**。否则「探测结束」
///    可能只是客户端关了管道、对方顺势退了，压根没验证到 kill。
/// 2. 每条测试用**自己的**标记串去 `pgrep` —— cargo test 默认多线程跑，
///    用 `pgrep -f quill-mcp-stub` 会把**别的测试**正在探测的子进程一起算进来，
///    于是这条测试在别处失败、这里误报（或者反过来，永远测不到自己的泄漏）。
#[tokio::test]
async fn a_finished_probe_leaves_no_child_process_behind() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let marker = format!("quill-probe-marker-{}", SEQ.fetch_add(1, Ordering::SeqCst));

    let r = row("cleanup", &stub(), &["--hold-open", &marker]);
    let p = mcp_client::probe(&r).await;
    assert!(
        p.connected,
        "这一条要先真的连上，才有「连完再断开」可测：{p:?}"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        // `pgrep` 不在这台机器上（Windows 上就没有）时跳过，而不是假装通过。
        let Ok(out) = std::process::Command::new("pgrep")
            .args(["-f", &marker])
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if text.trim().is_empty() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "探测结束后子进程还活着（pid {}）：传输层没有回收它",
            text.trim()
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// --------------------------------------------------------------- tools/call
//
// 上面那组盯的是「连得上、报得出工具」。下面这组盯的是**真的调起来**：
// 参数有没有过线、失败有没有被当成成功、非文本有没有被悄悄吞掉。
// 全部对着真子进程跑，没有 mock。

/// 真的调一次：参数**原样回到正文里**。
///
/// 盯的是「参数真的过了线」这件事被**观察到**，而不是靠「调用没报错」间接推断 ——
/// 一个把 arguments 整个丢掉的实现同样不会报错。这条是这个文件里唯一
/// 能区分「调通了」与「调了个空壳」的地方。
#[tokio::test]
async fn the_arguments_really_cross_the_wire_and_come_back_echoed() {
    let r = row("echo", &stub(), &["--echo-args"]);
    let out = mcp_client::call_tool(
        &r,
        "u1",
        "read-note",
        &serde_json::json!({
            "path": "会议纪要.md",
            "页码": 3
        }),
    )
    .await
    .expect("tools/call 应当成功");
    assert!(
        out.contains("read-note"),
        "正文里要能看到被调用的工具名：{out}"
    );
    assert!(
        out.contains("会议纪要.md"),
        "参数内容必须真的到了对端：{out}"
    );
    assert!(out.contains('3'), "数字参数也要过去：{out}");
    // 挂载名与远端原名在这里必须是**不同的两个东西**：协议里发出去的一直是原名，
    // 发挂载名的话服务器会回一个「没有这个工具」。
    assert!(
        !out.contains("echo__read-note"),
        "协议里必须用远端原名，不能用挂载名：{out}"
    );
}

/// `isError: true` 走 **Err**。走 Ok 的话模型会把失败当成功，接着编结论。
#[tokio::test]
async fn an_is_error_result_comes_back_as_a_failure_with_the_servers_own_words() {
    let r = row("boom", &stub(), &["--call-error"]);
    let err = mcp_client::call_tool(&r, "u1", "read-note", &serde_json::json!({}))
        .await
        .expect_err("isError=true 必须当失败");
    assert!(err.contains("故意失败"), "错误里要带服务器自己的话：{err}");
}

/// 只回 `structuredContent`、一条文本都没有时，**不能报成「什么都没返回」**。
#[tokio::test]
async fn a_structured_only_result_is_still_handed_to_the_model() {
    let r = row("struct", &stub(), &["--call-structured"]);
    let out = mcp_client::call_tool(&r, "u1", "count-things", &serde_json::json!({}))
        .await
        .expect("应当成功");
    assert!(out.contains('7'), "结构化结果要变成模型能读的文本：{out}");
    assert!(out.contains("甲"), "结构化结果的内容不能丢：{out}");
    assert!(
        !out.contains("没有返回任何内容"),
        "有结构化结果时不许说「没有返回任何内容」：{out}"
    );
}

/// 非文本块**明说**，不许悄悄吞掉。吞掉的话模型会以为工具什么都没返回。
#[tokio::test]
async fn a_non_text_block_is_reported_rather_than_silently_dropped() {
    let r = row("img", &stub(), &["--call-image"]);
    let out = mcp_client::call_tool(&r, "u1", "shot", &serde_json::json!({}))
        .await
        .expect("应当成功");
    assert!(out.contains("上面那张图"), "文本块要照给：{out}");
    assert!(
        out.contains("这一层还不认识的内容") || out.contains("图片"),
        "非文本块必须明说：{out}"
    );
}

/// 服务器在 `tools/call` 时退出：**不能报成功**，也不能把空字符串当结果。
#[tokio::test]
async fn a_server_that_dies_on_tools_call_is_not_reported_as_success() {
    let r = row("died", &stub(), &["--call-die"]);
    let err = mcp_client::call_tool(&r, "u1", "read-note", &serde_json::json!({}))
        .await
        .expect_err("子进程在调用时退出，必须当失败");
    assert!(!err.is_empty(), "失败也要说清是什么错：{err}");
    assert!(
        err.contains("下一步"),
        "面向用户的错误必须带「下一步」：{err}"
    );
}

/// **同步桥在完全没有运行时的线程上也能用。**
///
/// 这条是桥本身的验收：`call_tool_blocking` 是给同步的 `ToolHandler` 用的，
/// 而 `ToolHandler` 在对话链路里跑在一个**已经有 tokio 运行时**的线程上。
/// 如果这里只写 `#[tokio::test]`，那么「用 `Handle::block_on` 也能过」这种实现
/// 同样能过测试 —— 而它在 current-thread 运行时（`#[tokio::test]` 的默认值）
/// 上会 panic，在真服务的 multi-thread 上没事。这种错只有**没有运行时**
/// 的线程才抓得到，所以这里刻意用裸 `#[test]`。
#[test]
fn the_sync_bridge_works_on_a_thread_with_no_runtime_at_all() {
    let r = row("bridge", &stub(), &["--echo-args"]);
    let out =
        mcp_client::call_tool_blocking(&r, "u1", "read-note", &serde_json::json!({"path": "x.md"}))
            .expect("没有运行时的线程上也要能调通");
    assert!(out.contains("x.md"), "参数要真的过线：{out}");
}

/// 挂载名归一后撞车的两个工具：**第二个被跳过，不许顶掉第一个**。
///
/// 撞名在真实服务器上完全可能。`sanitize` 的两条规则各自都能造出撞名：
/// 连续的 `-` 合成一个，于是 `read--note` 与 `read-note` 一样；点、冒号、空格
/// 直接丢掉，于是 `read.note` 与 `readnote` 一样。这里用第一对。
///
/// 盯的是**跳过**而不是顶掉：顶掉的话模型看到的是 `read-note` 的描述里写着
/// 「带连字符的工具名」，而用户配的是另一个 —— 症状会出现在模型答错内容上，
/// 离真正的原因隔了三层。
#[tokio::test]
async fn two_tools_that_normalize_to_the_same_name_do_not_overwrite_each_other() {
    use quill_server::mcp_client::RemoteTool;
    use quill_server::tools::{
        mcp_tool_name, mcp_tool_spec, mcp_tool_visibility, McpToolVisibility,
    };

    let mk = |n: &str| RemoteTool {
        remote_name: n.to_string(),
        description: format!("{n} 的说明"),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    };
    assert_eq!(
        mcp_tool_name("srv", "read--note"),
        mcp_tool_name("srv", "read-note"),
        "这两个名字归一后必须真的相同，否则这条测试测的不是撞名"
    );
    // 另一条规则也要真的成立，否则「点被丢掉」这件事没人盯。
    assert_eq!(
        mcp_tool_name("srv", "read.note"),
        mcp_tool_name("srv", "readnote"),
        "丢掉分隔符这条规则也必须真的把两个名字并到一起"
    );

    let first = mcp_tool_spec("srv", &mk("read--note"));
    let taken = vec![first.clone()];
    match mcp_tool_visibility(&taken, "srv", &mk("read-note")) {
        McpToolVisibility::NotMounted(why) => {
            assert!(why.contains("顶掉"), "原因要说清是「会顶掉」：{why}")
        }
        other => panic!("撞名了却判成可挂载，第二个会顶掉第一个：{other:?}"),
    }
    assert_eq!(
        taken[0].name, first.name,
        "跳过的语义是「不动它」，不是「换掉它」"
    );
}

/// **服务器自报「我没有 tools 能力」时，工具数按 0 报**，哪怕它的 `tools/list`
/// 真的回了内容。ISSUE-014 的回归。
///
/// 这条盯的是「本地开关」与「服务器自报」**两道闸门都要过**。只看本地那道的话，
/// 这台服务器会被挂出 3 个工具，界面上写「模型调得到」，而模型调过去只会拿到
/// 一个「方法不存在」—— 界面上没有任何迹象说明这一点。
///
/// 顺带把 `--caps-off` 这个夹具用起来：它的用法注释里一直写着这个开关，
/// 但整个测试套件里没有任何一条测试用到它（见 ISSUE-014 的「为什么一直没被抓到」）。
#[tokio::test]
async fn a_server_that_self_reports_no_tools_capability_reports_zero_not_its_list() {
    let r = as_stub("selfoff", &["--caps-off"]);
    // 先确认这个夹具真的造出了「自报没有 tools、但 tools/list 照样回内容」：
    // 下面那两条断言的力度全靠它，否则这条测试可能只是量了一个空壳。
    let d = mcp_client::discover(&r).await;

    assert!(d.probe.probed, "确实发起了握手：{d:?}");
    assert!(
        d.probe.connected,
        "握手本身是成功的 —— 这是正常状态，不是连接失败：{d:?}"
    );
    assert_eq!(
        d.probe.server_declares_tools,
        Some(false),
        "服务器自报的能力里没有 tools，必须被读出来：{d:?}"
    );
    assert_eq!(
        d.probe.tool_count, 0,
        "自报没有 tools 的服务器，工具数必须是 0：{d:?}"
    );
    assert!(
        d.tools.is_empty(),
        "自报没有 tools 的服务器，一个工具都不许挂进工具表：{d:?}"
    );
    // `connected=true` 但工具数 0 的原因必须写清楚，且要给「下一步」。
    let why = d.probe.error.as_deref().expect("工具数是 0 必须有原因");
    assert!(why.contains("没有 tools"), "{why}");
    assert!(why.contains("下一步"), "{why}");
}

/// 反过来：自报**有** tools 能力的服务器，本地开关也开着，就该正常挂上。
/// 与上一条成对 —— 只测「按 0」的话，一个「永远返回 0」的实现也能过。
#[tokio::test]
async fn a_server_that_declares_tools_is_mounted_normally() {
    let d = mcp_client::discover(&as_stub("selfon", &[])).await;
    assert_eq!(d.probe.server_declares_tools, Some(true), "{d:?}");
    assert_eq!(d.probe.tool_count, 3, "{d:?}");
    assert_eq!(d.tools.len(), 3, "{d:?}");
    assert_eq!(d.probe.error, None, "一切正常就不该带原因：{d:?}");
}

/// 自报没有 tools 的服务器，**直接调它的工具**也要被挡在门外。
///
/// 与挂载侧同一道闸门。挂载侧按 0 处理之后，模型正常情况下调不到它；这条盯的是
/// 「万一挂上了呢」——那时模型拿到的是一个看不懂的协议错，而不是一句白话。
#[tokio::test]
async fn calling_a_tool_on_a_server_that_declares_none_is_refused_with_a_next_step() {
    let r = as_stub("selfoff", &["--caps-off"]);
    let err = mcp_client::call_tool(&r, "u1", "read-note", &serde_json::json!({}))
        .await
        .expect_err("自报没有 tools 的服务器，调用必须被挡住");
    assert!(err.contains("没有 tools"), "{err}");
    assert!(err.contains("下一步"), "{err}");
}

/// 界面上报的「挂了几个」必须与 `with_mcp_tools` 真的挂上去的**逐个相等**。
///
/// 这一对是本轮的核心防线：界面说 3 个、模型那轮只调得到 2 个，
/// 用户没有任何办法发现 —— 除非有一条测试把两边**真的**跑一遍再对齐。
#[tokio::test]
async fn the_discovery_tool_list_is_exactly_what_the_mount_decision_sees() {
    use quill_server::tools::{mcp_tool_name, mcp_tool_spec, mcp_tool_visibility};

    let r = row("notes", &stub(), &[]);
    let d = mcp_client::discover(&r).await;
    assert!(d.probe.connected, "先要真的连上：{d:?}");
    // 分页必须翻完：stub 首页 2 个 + 游标后 1 个。少翻一页的话这里只有 2，
    // 而 2 看上去也挺像回事。
    assert_eq!(d.tools.len(), 3, "工具列表必须翻完分页：{d:?}");
    assert_eq!(
        d.tools.len(),
        d.probe.tool_count,
        "条数与上报的必须一致：{d:?}"
    );

    // 拿一个空的基线表，按 `with_mcp_tools` 的顺序走一遍判定。
    let mut taken: Vec<quill_provider::ToolSpec> = Vec::new();
    let mut mounted = Vec::new();
    for tool in &d.tools {
        if mcp_tool_visibility(&taken, &d.probe.name, tool).model_can_see() {
            let spec = mcp_tool_spec(&d.probe.name, tool);
            mounted.push(spec.name.clone());
            taken.push(spec);
        }
    }
    let expected: Vec<String> = d
        .tools
        .iter()
        .map(|t| mcp_tool_name(&d.probe.name, &t.remote_name).expect("名字应可归一"))
        .collect();
    assert_eq!(
        mounted, expected,
        "三个工具都该挂上，且挂载名与原名一一对应"
    );
}
