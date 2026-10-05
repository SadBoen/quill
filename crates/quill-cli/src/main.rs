//! `quill` 可执行入口 —— **只做路由**，逻辑全在 `quill_cli` 库里。
//!
//! 拆成 lib + bin 的原因不是洁癖：`tests/` 下的集成测试**够不到 bin crate
//! 的内部**，而「读操作不得静默建号」这类性质只能从外部验。

use quill_cli::{dispatch, Outcome, USAGE};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let out: Outcome = dispatch(&args).await;
    if !out.message.is_empty() {
        match out.code {
            0 => println!("{}", out.message),
            // ▲ 单独一档：让「查不出来」在屏幕上比「失败」更显眼
            2 => eprintln!("▲ {}", out.message),
            _ => eprintln!("✗ {}", out.message),
        }
    }
    ExitCode::from(out.code)
}
