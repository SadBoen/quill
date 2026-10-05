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
    assert!(
        why.contains("initialize"),
        "要说清卡在哪一步：{why}"
    );
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
    assert!(
        why.contains("手动跑一遍"),
        "错误要带得动手的下一步：{why}"
    );
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
    assert!(p.connected, "这一条要先真的连上，才有「连完再断开」可测：{p:?}");

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
