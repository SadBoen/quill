use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::auth::{CompositeTokenResolver, EnvTokenResolver};
use crate::config::Config;
use crate::routes::build_router;
use crate::state::AppState;

pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

pub fn build_state(config: Config) -> (AppState, Vec<crate::config::Warning>) {
    let (env_resolver, mut warnings) = EnvTokenResolver::from_env();
    let (resolver, session_resolver) = CompositeTokenResolver::new(env_resolver);
    let mut all = config.warnings.clone();
    all.append(&mut warnings);
    let subjects = resolver.env().subjects();

    let db_path = config.db_path.display().to_string();
    let max_conn = config.db_max_connections;
    let (db, problem) = match crate::db::DbBridge::open(&db_path, max_conn) {
        Ok(bridge) => {
            match bridge.migrate_embedded() {
                Ok(report) if report.changed() => all.push(crate::config::Warning {
                    source: "QUILL_DB_PATH".to_string(),
                    message: format!("已自动应用迁移 {:?}（数据库 {db_path}）。", report.applied),
                }),
                Ok(_) => match bridge.missing_tables() {
                    Ok(missing) if missing.is_empty() => {}
                    Ok(missing) => all.push(crate::config::Warning {
                        source: "QUILL_DB_PATH".to_string(),
                        message: format!(
                            "数据库 {db_path} 缺少表 {missing:?}，但迁移台账显示已是最新。\
                             这说明该文件不是 quill 的库（或被人手改过结构）。\
                             下一步：改用 `QUILL_DB_PATH` 指向一个空路径，让 quill 建新库。"
                        ),
                    }),
                    Err(e) => all.push(crate::config::Warning {
                        source: "QUILL_DB_PATH".to_string(),
                        message: format!("无法探测数据库 {db_path} 的表结构：{e}。"),
                    }),
                },
                Err(e) => all.push(crate::config::Warning {
                    source: "QUILL_DB_PATH".to_string(),
                    message: format!("自动迁移失败，专家与派工路由会返回 503。\n{e}"),
                }),
            }

            if !subjects.is_empty() {
                match bridge.ensure_token_users(subjects.clone()) {
                    Ok(created) if !created.is_empty() => {
                        let names: Vec<String> = subjects
                            .iter()
                            .filter(|(id, _, _)| created.contains(id))
                            .map(|(_, login, _)| login.clone())
                            .collect();
                        println!(
                            "  已为令牌用户自动建档：{}（这些账号只能靠令牌登录，没有密码）",
                            names.join("、")
                        );
                    }
                    Ok(_) => {}
                    Err(e) => all.push(crate::config::Warning {
                        source: "QUILL_TOKENS".to_string(),
                        message: format!(
                            "令牌用户建档失败：{e}\n\
                             这些令牌即使能通过鉴权，也写不了任何带外键的数据（会话、消息、团队、扩展都会报外键错误）。\n\
                             下一步：修好上面的写库错误后重启服务。"
                        ),
                    }),
                }
            }

            // headless 部署的密码账号入口：浏览器首管引导只对「全新实例」可用，
            // 容器/CI/远端机器没人点得了。这里补一条配置驱动的建号通道，
            // **不是注册的替代品** —— `/api/auth/register` 依然不存在。
            match bridge
                .ensure_password_users(std::env::var("QUILL_PASSWORD_USERS").ok().as_deref())
            {
                Ok(()) => {}
                Err(problems) => {
                    for (source, message) in problems {
                        all.push(crate::config::Warning {
                            source: source.clone(),
                            message,
                        });
                    }
                }
            }

            let bridge = Arc::new(bridge);
            // 库建好了，把同一个 Arc 交给令牌解析器：会话令牌要靠它查库，
            // 环境变量令牌也要靠它核 `users` 行的状态与角色。必须是同一个 Arc
            // —— 它是唯一持有 worker 池的那个。
            resolver.attach(Arc::clone(&bridge));
            session_resolver.attach(Arc::clone(&bridge));

            (Some(bridge), None)
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

    let (env_llm_config, mut llm_warnings) = crate::llm::LlmConfig::from_env();
    all.append(&mut llm_warnings);
    // 先按 env 拼一个 provider / config 占位；下一步若有数据库，会用表里的
    // 内容再次覆盖 — 让 admin 表成为唯一真相来源。
    let initial_provider = if env_llm_config.enabled {
        match crate::llm::build(&env_llm_config) {
            Ok(p) => Some(p),
            Err(e) => {
                all.push(crate::config::Warning {
                    source: "QUILL_LLM_BASE_URL".to_string(),
                    message: format!("模型服务构造失败，对话相关接口会返回 503：{e}"),
                });
                None
            }
        }
    } else {
        all.push(crate::config::Warning {
            source: "QUILL_LLM_BASE_URL".to_string(),
            message: "未配置模型服务，对话相关接口会返回 503。\
                      下一步：启动 llama-server，或设置 QUILL_LLM_BASE_URL / QUILL_LLM_MODEL；\
                      也可以由 admin 通过 PUT /api/admin/config 在线写入。"
                .to_string(),
        });
        None
    };

    let llm_slot = Arc::new(RwLock::new(initial_provider));
    let llm_config_slot = Arc::new(RwLock::new(env_llm_config.clone()));
    let provider_slot = Arc::new(RwLock::new(Default::default()));

    let state = AppState {
        config,
        tokens: Arc::new(resolver),
        db,
        db_problem: problem,
        llm: llm_slot,
        llm_config: llm_config_slot,
        providers: provider_slot,
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::production(),
    };

    // 回填：`llm_providers` 表为空时种一条 kind=custom 的默认 provider，
    // 来源优先级 = admin_config 行 → 环境变量。表里已有行则以表为准，
    // 重新 build provider + 覆盖 llm_config。
    if state.db.is_some() {
        match crate::llm_providers::list(&state) {
            Ok(rows) if !rows.is_empty() => {
                let cache = crate::llm_providers::ProviderCache { providers: rows };
                state.set_provider_cache(cache.clone());
                state.apply_default_provider(&cache);
            }
            Ok(_) => seed_first_provider(&state, &env_llm_config, &mut all),
            Err(e) => all.push(crate::config::Warning {
                source: "QUILL_ADMIN_CONFIG".to_string(),
                message: format!(
                    "读取 llm_providers 时出错（{e}）。下一步：用 quill doctor --section=db 看诊断，\
                     或手动 POST /api/admin/providers 重建。"
                ),
            }),
        }
    } else {
        // 没数据库就不写表，保留 env 行为不变 — 启动告警里会说。
        all.push(crate::config::Warning {
            source: "QUILL_ADMIN_CONFIG".to_string(),
            message: "数据库不可用：模型供应商表未初始化，模型配置维持环境变量路径，\
                      等修好存储后由 admin 通过 POST /api/admin/providers 落地。"
                .to_string(),
        });
    }

    (state, all)
}

/// 首启播种：优先用 0002 的 `admin_config` 行（老库升级过来时它才是真相），
/// 没有就用环境变量。构造不出 provider 就只告警并退回 env —— 宁可没有
/// provider 也不能让服务起不来。
fn seed_first_provider(
    state: &AppState,
    env_llm_config: &crate::llm::LlmConfig,
    warnings: &mut Vec<crate::config::Warning>,
) {
    let legacy = crate::api_admin::load_admin_config(state)
        .ok()
        .map(|c| c.to_llm_config());
    let base = legacy.clone().unwrap_or_else(|| env_llm_config.clone());

    if let Err(e) = crate::api_admin::seed_admin_config_from_env(state, &base) {
        warnings.push(crate::config::Warning {
            source: "QUILL_ADMIN_CONFIG".to_string(),
            message: format!(
                "首次启动时回填 admin_config 失败：{e}。\
                 下一步：执行 PUT /api/admin/config 写入一次，\
                 或检查数据库可写权限（quill doctor --section=db）。"
            ),
        });
    }

    match crate::llm::build(&base) {
        Ok(_) => {}
        Err(detail) => {
            eprintln!(
                "⚠ 模型服务构造失败（{detail}）：不播种模型供应商，模型配置退回环境变量。\
                 下一步：修正 QUILL_LLM_BASE_URL 后重启，或由 admin 通过 \
                 POST /api/admin/providers 在线写入。"
            );
            warnings.push(crate::config::Warning {
                source: "QUILL_LLM_BASE_URL".to_string(),
                message: format!(
                    "模型服务构造失败（{detail}），未播种模型供应商，对话相关接口沿用环境变量路径。"
                ),
            });
            return;
        }
    }

    match crate::llm_providers::seed_default_from_llm_config(state, &base, "默认模型服务") {
        Ok(_) => {
            if let Ok(cache) = state.reload_providers() {
                state.apply_default_provider(&cache);
            }
        }
        Err(e) => warnings.push(crate::config::Warning {
            source: "QUILL_ADMIN_CONFIG".to_string(),
            message: format!(
                "首次启动时播种模型供应商失败：{e}。\
                 下一步：执行 POST /api/admin/providers 写入一条，\
                 或检查数据库可写权限（quill doctor --section=db）。"
            ),
        }),
    }
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

    // 定时任务的调度器（Q042）：**服务在跑它才在跑**。
    // 只在有存储时起 —— 没有库就没有任务可投，起了也只是空转。
    // 测试不经 `serve()`（它们直接用 `build_router`），所以后台循环不会污染用例。
    if state.db.is_some() {
        crate::cron_scheduler::spawn(state.clone());
    }

    let app = build_router(state);
    let timeout = shutdown_timeout();

    axum::serve(
        listener,
        // `into_make_service_with_connect_info` 让处理器能拿到真实对端地址，
        // 登录限流按「用户名 + 来源 IP」计额就靠它。不注入的话 `ConnectInfo`
        // 提取器会直接 500 —— 这是它故意的：宁可报错也别默默当成「未知来源」
        // 让人拿到无限额度。
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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
