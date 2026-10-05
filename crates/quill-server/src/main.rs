//! Quill 服务端二进制入口 —— 依赖图的顶端。
//!
//! # 职责
//!
//! 组装全部 crate、启动 axum 服务、注册中间件链、优雅关闭。
//!
//! # ⚠️ 依赖方向铁律（契约 §一）
//!
//! 本 crate 是**唯一可以依赖全部 quill crate 的地方**。
//! 任何其他 `quill-*` crate 依赖它都会形成环；
//! 该约束由 `scripts/check-crate-deps.sh`（G40）执行。
//!
//! # 失败语义（铁律七）
//!
//! - **配置错**（`QUILL_ADDR` / `QUILL_WEB_DIR` / `QUILL_TOKENS` 非法）→ 打 WARN +
//!   回退默认值，**服务照常起**。理由：本项目无人工值守，配置错就起不来
//!   等于让 `quill doctor` 这条唯一诊断路径也连不上。
//! - **端口占用 / 绑定失败** → 打印含可复制排查命令的中文错误，非零退出。
//!   这类错误无法回退（没有"默认端口"能绕开占用），必须让用户立刻看到。

use quill_server::config::Config;
use quill_server::server;

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let addr = config.addr;
    let (state, warnings) = server::build_state(config);

    // ⚠️ 启动期告警**逐条打印**：铁律十六要求"无法判定/降级"的数量必须显眼，
    //    折叠成一行摘要等于把它藏起来。
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
