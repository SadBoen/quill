
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

            2 => eprintln!("▲ {}", out.message),
            _ => eprintln!("✗ {}", out.message),
        }
    }
    ExitCode::from(out.code)
}
