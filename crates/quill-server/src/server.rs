
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::EnvTokenResolver;
use crate::config::Config;
use crate::routes::build_router;
use crate::state::AppState;

pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

pub fn build_state(config: Config) -> (AppState, Vec<crate::config::Warning>) {
    let (resolver, mut warnings) = EnvTokenResolver::from_env();
    let mut all = config.warnings.clone();
    all.append(&mut warnings);

    let db_path = config.db_path.display().to_string();
    let max_conn = config.db_max_connections;
    let (db, problem) = match crate::db::DbBridge::open(&db_path, max_conn) {
        Ok(bridge) => {

            match bridge.missing_tables() {
                Ok(missing) if missing.is_empty() => {}
                Ok(missing) => all.push(crate::config::Warning {
                    source: "QUILL_DB_PATH".to_string(),
                    message: format!(
                        "数据库 {db_path} 缺少表 {:?}：schema 未迁移，\
                         专家与派工路由会返回 503。\
                         下一步：按 crates/quill-store/migrations/0001_init.sql 执行迁移\
                         （quill-upgrade 目前仍是骨架，尚无 CLI 入口），\
                         迁移后用 `quill doctor --section=db` 复核。",
                        missing
                    ),
                }),
                Err(e) => all.push(crate::config::Warning {
                    source: "QUILL_DB_PATH".to_string(),
                    message: format!(
                        "无法探测数据库 {db_path} 的表结构：{e}。\
                         下一步：执行 `quill doctor --section=db` 查看完整诊断。"
                    ),
                }),
            }
            (Some(Arc::new(bridge)), None)
        }
        Err(e) => {
            let reason = e.to_string();
            all.push(crate::config::Warning {
                source: "QUILL_DB_PATH".to_string(),
                message: format!(
                    "存储不可用（{db_path}）：{reason}。\
                     服务继续启动，但专家与派工路由会返回 503。\
                     下一步：先 `mkdir -p` 父目录并确认可写，再执行 `quill doctor --section=db`。"
                ),
            });
            (None, Some(reason))
        }
    };

    (
        AppState {
            config,
            tokens: Arc::new(resolver),
            db,
            db_problem: problem,
        },
        all,
    )
}

pub async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        format!(
            "无法绑定监听地址 {addr}：{e}。\
             端口可能已被占用（执行 `ss -ltnp | grep {port}` 查看占用进程）。\
             换一个端口重试：设置环境变量 QUILL_ADDR=127.0.0.1:{alt} 后重新启动。",
            port = addr.port(),
            alt = addr.port() + 1,
        )
    })
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), String> {
    let listener = bind(addr).await?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("无法读取实际监听地址：{e}"))?;

    println!("quill-server 已监听 http://{local}");
    println!("  健康检查：curl http://{local}/healthz");
    match &state.db {
        Some(db) => println!(
            "  数据库：{}（就绪与否见 /healthz 的 storage 字段）",
            db.path()
        ),
        None => println!(
            "  ⚠ 数据库不可用：{}（专家与派工路由将返回 503）",
            state.db_problem.as_deref().unwrap_or("原因未记录")
        ),
    }
    if !state.config.ui_assets_available() {
        println!(
            "  ⚠ 前端产物目录 {} 不可用 —— 后端 API 正常，前端页面不可访问",
            state.config.web_dir.display()
        );
    }
    for w in &state.config.warnings {
        println!("  ⚠ WARN {w}");
    }

    let app = build_router(state);
    let timeout = shutdown_timeout();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            println!("收到关闭信号：停止接受新连接，等待在途请求完成（上限 {timeout:?}）…");
        })
        .await
        .map_err(|e| format!("HTTP 服务异常退出：{e}"))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("⚠ 无法注册 SIGTERM 处理器（{e}），只等 Ctrl-C");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {

        let _ = tokio::signal::ctrl_c().await;
    }
}

fn shutdown_timeout() -> Duration {
    let secs = std::env::var("QUILL_SHUTDOWN_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT.as_secs());
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_failure_returns_chinese_actionable_message_not_panic() {

        let first = bind("127.0.0.1:0".parse().expect("合法地址")).await;
        let Ok(listener) = first else {

            eprintln!("本环境无法绑定回环端口，跳过占用冲突用例");
            return;
        };
        let taken = listener.local_addr().expect("已绑定");
        let err = bind(taken).await.expect_err("同一端口二次绑定必须失败");
        assert!(err.contains("ss -ltnp"), "错误必须给出可复制的排查命令");
        assert!(err.contains("QUILL_ADDR"), "错误必须给出可复制的修复命令");
    }

    #[test]
    fn shutdown_timeout_falls_back_when_env_is_garbage() {

        let t = shutdown_timeout();
        assert!(t >= Duration::from_secs(1), "超时必须至少 1 秒");
    }

    #[tokio::test]
    async fn build_state_merges_config_and_token_warnings_without_loss() {

        let mut config = Config::from_env();
        config.warnings.push(crate::config::Warning {
            source: "QUILL_ADDR".to_string(),
            message: "测试注入的告警".to_string(),
        });
        let expected = config.warnings.len();

        let (_state, warnings) = build_state(config);

        assert!(
            warnings.len() >= expected,
            "config 侧告警被丢掉了：期望至少 {expected} 条，实际 {} 条",
            warnings.len()
        );
        assert!(
            warnings.iter().any(|w| w.message == "测试注入的告警"),
            "合并后必须仍能找到 config 侧那条告警"
        );
    }
}
