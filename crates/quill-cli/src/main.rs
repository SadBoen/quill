use quill_cli::{dispatch, Outcome, USAGE};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // `quill mcp <server>`：跑一台 **stdio MCP 服务器**，照 goose 的 `goose mcp <key>`
    // （`goose-mcp/src/mcp_server_runner.rs` + `goose-cli` 的 `Command::Mcp`）。
    //
    // **必须在 `dispatch` 之前拦下来**：`dispatch` 返回 `Outcome`，`main` 会把它的
    // message 打到 **stdout**；而 stdio MCP 服务器的 stdout 只能出协议内容，
    // 多打一行客户端就解析失败。所以这条路径不走 Outcome、也不往 stdout 打任何字。
    if args[0] == "mcp" {
        return run_mcp(&args[1..]).await;
    }

    let out: Outcome = dispatch(&args).await;
    if !out.message.is_empty() {
        match out.code {
            0 => println!("{}", out.message),

            2 => eprintln!("▲ {}", out.message),
            _ => eprintln!("✗ {}", out.message),
        }
    }
    ExitCode::from(out.code)
}

/// `quill mcp <server>` 的分派。目前只有 `memory`（与 goose 的 `McpCommand` 里 quill
/// 已移植的那一支对应）。
async fn run_mcp(rest: &[String]) -> ExitCode {
    match rest.first().map(String::as_str) {
        Some("memory") => match quill_core::memory::serve_stdio().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                // 失败信息走 stderr —— stdout 是协议通道。
                eprintln!("✗ {e}");
                ExitCode::from(1)
            }
        },
        other => {
            eprintln!(
                "✗ 不认识的 MCP 服务器 {other:?}。目前可用：memory\
                 （`quill --help` 有说明）"
            );
            ExitCode::from(1)
        }
    }
}
