
use quill_server::config::Config;
use quill_server::server;

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let addr = config.addr;
    let (state, warnings) = server::build_state(config);

    if !warnings.is_empty() {
        eprintln!(
            "⚠ 启动告警 {} 条（配置已回退默认值，服务继续启动）：",
            warnings.len()
        );
        for w in &warnings {
            eprintln!("  · {w}");
        }
    }

    if let Err(msg) = server::serve(state, addr).await {
        eprintln!("✗ quill-server 启动失败：{msg}");
        std::process::exit(1);
    }
    println!("quill-server 已优雅退出。");
}
