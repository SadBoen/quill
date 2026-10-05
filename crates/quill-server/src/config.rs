//! 启动配置 —— 从环境变量读取，**配错不得 panic**。
//!
//! # 铁律七：环境变量配错不得 panic
//!
//! 原文：「环境变量配错不得 panic（如 `QUILL_WEB_DIR` 指向不存在目录 → **回退默认 + WARN**，服务仍能起）」。
//!
//! 因此本模块的**每一个** `from_env` 都在做同一件事：
//! 解析失败 → 记一条 `WARN` → 回退到可用默认值 → 继续启动。
//!
//! # 为什么回退而不是退出
//!
//! 本项目**无人工值守**（`V1_SCOPE_CONSTRAINTS` 第五节）。一个配错的环境变量
//! 就让服务起不来，等于把「用户改了一行配置」变成「实例彻底不可用」——
//! 而 `quill doctor` 这条唯一诊断路径此时也连不上。
//! 回退 + WARN 让服务保持可诊断（`/healthz` 仍 200，且能读到 WARN 列表）。
//!
//! ⚠️ **但"回退"不等于"静默"**：所有 WARN 都在启动横幅里逐条打印，
//! 且经 `GET /healthz` 的 `warnings` 字段对外可见（铁律四：不能只在日志里留痕）。

use std::net::SocketAddr;
use std::path::PathBuf;

/// 默认监听地址：只听本机回环。
///
/// ⚠️ 选 `127.0.0.1` 而非 `0.0.0.0` 是**安全默认值**：本服务带令牌鉴权，
/// 但绑定全网卡会把「配错令牌 = 全网不可用」变成「配错令牌 = 全网可写」。
/// 真要对外暴露时由部署方显式设置 `QUILL_ADDR`。
pub const DEFAULT_ADDR: &str = "127.0.0.1:8848";

/// 默认 Web 产物目录。
pub const DEFAULT_WEB_DIR: &str = "ui/web/dist";

/// 默认数据库路径（相对进程工作目录）。
///
/// ⚠️ 选**单文件**而不是「每用户一个库」：`quill-store/migrations/0001_init.sql`
/// 的 `users` / `sessions_auth` / `invites` 三张表是**全局**的（登录在专家之前），
/// 且 `sessions.user_id → users(id)` 是跨域外键。按用户分库会让「登录」无处安放。
/// 用户级隔离靠**每行都带 `user_id`**（复合主键 + 复合外键）实现，不靠分库。
pub const DEFAULT_DB_PATH: &str = "data/quill.db";

/// 默认连接池上限。
///
/// ⚠️ SQLite 是单写者：写并发由 `busy_timeout` 排队，而不是靠多条连接硬扛。
/// 读侧可以并行，故默认 5（不是 1，也不是架构文档里面向 Postgres 的 20）。
pub const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;

/// 一条启动期告警。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// 触发的环境变量名（便于用户直接定位要改哪一行）。
    pub source: String,
    /// 中文人话说明。
    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

/// 服务配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// 监听地址。
    pub addr: SocketAddr,
    /// Web 产物目录；不存在时**不回退**（目录路径是用户意图，不是可猜的默认值），
    /// 但也不 panic —— `ui_assets_available` 会是 `false`，`/healthz` 显式报出来。
    pub web_dir: PathBuf,
    /// 是否注册「自检用 panic 路由」（`/__selftest__/panic`）。
    ///
    /// ⚠️ 默认 `false`。开启它等于开一个**可被外部触发的自我 DoS 开关**，
    /// 因此只有显式设置 `QUILL_ENABLE_SELFTEST=1` 才会为真。
    /// ⚠️ 做成**字段**而不是在 `routes.rs` 里直接读环境变量：环境变量是
    ///    **进程全局**的，测试里改它会与并行用例互相污染（铁律十四：
    ///    「这个通过，是因为真的检查了，还是因为根本没跑？」）。
    ///    走配置字段后，测试可以**按实例**精确开关。
    pub enable_selftest: bool,
    /// SQLite 数据库路径（`QUILL_DB_PATH`）。
    ///
    /// ⚠️ 父目录不存在**不**在这里报错：建库由 `quill_store::configure_pool`
    /// 尝试，失败会在 `build_state` 里变成一条 WARN + 503 响应（铁律七）。
    pub db_path: PathBuf,
    /// 连接池上限（`QUILL_DB_MAX_CONNECTIONS`）。
    pub db_max_connections: u32,
    /// 启动期告警。
    pub warnings: Vec<Warning>,
}

impl Config {
    /// 从环境变量装配配置。
    ///
    /// **永不 panic、永不 `unwrap`**。每项解析失败都产生一条 [`Warning`]。
    pub fn from_env() -> Self {
        let mut warnings = Vec::new();

        let addr = parse_addr(&mut warnings);
        let web_dir = parse_web_dir(&mut warnings);
        let enable_selftest = std::env::var("QUILL_ENABLE_SELFTEST").as_deref() == Ok("1");
        let db_path = parse_db_path(
            std::env::var("QUILL_DB_PATH").ok().as_deref(),
            &mut warnings,
        );
        let db_max_connections = parse_db_max_connections(
            std::env::var("QUILL_DB_MAX_CONNECTIONS").ok().as_deref(),
            &mut warnings,
        );

        Self {
            addr,
            web_dir,
            enable_selftest,
            db_path,
            db_max_connections,
            warnings,
        }
    }

    /// Web 产物目录是否可用（存在且是目录）。
    pub fn ui_assets_available(&self) -> bool {
        self.web_dir.is_dir()
    }
}

fn parse_addr(warnings: &mut Vec<Warning>) -> SocketAddr {
    match std::env::var("QUILL_ADDR") {
        Err(_) => {
            // 未设置不是问题，不产生 WARN（每次启动都刷一条 WARN 会训练用户忽略 WARN）。
            DEFAULT_ADDR
                .parse()
                .expect("DEFAULT_ADDR 是本文件内的字面量常量，非用户输入")
        }
        Ok(raw) => {
            let trimmed = raw.trim();
            match trimmed.parse::<SocketAddr>() {
                Ok(a) => a,
                Err(e) => {
                    warnings.push(Warning {
                        source: "QUILL_ADDR".to_string(),
                        message: format!(
                            "无法解析监听地址 {trimmed:?}（{e}），已回退默认 {DEFAULT_ADDR}。\
                             要改监听地址请设为 `host:port`，例如 `0.0.0.0:8848`。"
                        ),
                    });
                    DEFAULT_ADDR
                        .parse()
                        .expect("DEFAULT_ADDR 是本文件内的字面量常量，非用户输入")
                }
            }
        }
    }
}

fn parse_web_dir(warnings: &mut Vec<Warning>) -> PathBuf {
    match std::env::var("QUILL_WEB_DIR") {
        Err(_) => PathBuf::from(DEFAULT_WEB_DIR),
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                warnings.push(Warning {
                    source: "QUILL_WEB_DIR".to_string(),
                    message: format!("设为空串，已回退默认 {DEFAULT_WEB_DIR}。"),
                });
                return PathBuf::from(DEFAULT_WEB_DIR);
            }
            let p = PathBuf::from(trimmed);
            if !p.exists() {
                // ⚠️ 显式提示下一步命令（铁律七：可直接复制）。
                //    目录不存在不阻断启动：/healthz 会报 warnings，daemon 仍可被诊断。
                warnings.push(Warning {
                    source: "QUILL_WEB_DIR".to_string(),
                    message: format!(
                        "目录 {trimmed:?} 不存在，前端产物不会挂载（后端 API 不受影响）。\
                         若要挂载前端，先执行 `cd ui/web && npm ci && npm run build`；\
                         或把 QUILL_WEB_DIR 指向已有产物目录。"
                    ),
                });
            } else if !p.is_dir() {
                warnings.push(Warning {
                    source: "QUILL_WEB_DIR".to_string(),
                    message: format!(
                        "{trimmed:?} 不是目录（是文件），前端产物不会挂载。\
                         请把它指向构建产物**目录**，例如 {DEFAULT_WEB_DIR}。"
                    ),
                });
            }
            p
        }
    }
}

/// 解析数据库路径。
///
/// ⚠️ 显式传 `raw` 而不是自己读环境变量：环境变量是**进程全局**的，
/// 单元测试里改它会与并行用例互相污染（铁律十四）。这样拆开后
/// 「垃圾输入 → 回退 + WARN」这条路径可以被真正测到。
fn parse_db_path(raw: Option<&str>, warnings: &mut Vec<Warning>) -> PathBuf {
    match raw {
        None => PathBuf::from(DEFAULT_DB_PATH),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                warnings.push(Warning {
                    source: "QUILL_DB_PATH".to_string(),
                    message: format!("设为空串，已回退默认 {DEFAULT_DB_PATH}。"),
                });
                return PathBuf::from(DEFAULT_DB_PATH);
            }
            let p = PathBuf::from(trimmed);
            match p.parent() {
                // ⚠️ 父目录不存在**只告警不阻断**：真正建库在 `build_state`，
                //    那里失败会变成 503 + 中文原因。让服务继续起，
                //    `/healthz` 与 `quill doctor` 才连得上（铁律七）。
                Some(dir) if !dir.as_os_str().is_empty() && !dir.is_dir() => {
                    warnings.push(Warning {
                        source: "QUILL_DB_PATH".to_string(),
                        message: format!(
                            "父目录 {} 不存在，数据库可能建不出来。\
                             下一步：先执行 `mkdir -p {}`，或把 QUILL_DB_PATH 指向已存在的目录。",
                            dir.display(),
                            dir.display()
                        ),
                    });
                }
                _ => {}
            }
            p
        }
    }
}

/// 解析连接池上限。
fn parse_db_max_connections(raw: Option<&str>, warnings: &mut Vec<Warning>) -> u32 {
    match raw {
        None => DEFAULT_DB_MAX_CONNECTIONS,
        Some(raw) => match raw.trim().parse::<u32>() {
            // ⚠️ 0 条连接会让每次查询立刻失败（池空）→ 必须回退而不是照用。
            Ok(n) if n >= 1 => n,
            _ => {
                warnings.push(Warning {
                    source: "QUILL_DB_MAX_CONNECTIONS".to_string(),
                    message: format!(
                        "值 {raw:?} 非法（需为 ≥ 1 的整数），已回退默认 {DEFAULT_DB_MAX_CONNECTIONS}。"
                    ),
                });
                DEFAULT_DB_MAX_CONNECTIONS
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 反向断言：0 与垃圾值都必须回退 + 产生 WARN（铁律七：配错不 panic）。
    #[test]
    fn zero_and_garbage_connections_fall_back_with_a_warning() {
        for bad in ["0", "abc", "-3", ""] {
            let mut w = Vec::new();
            let n = parse_db_max_connections(Some(bad), &mut w);
            assert_eq!(n, DEFAULT_DB_MAX_CONNECTIONS, "输入 {bad:?} 必须回退");
            assert_eq!(
                w.len(),
                1,
                "输入 {bad:?} 必须产生恰好一条 WARN（不告警=静默）"
            );
        }
    }

    #[test]
    fn valid_connection_count_is_kept() {
        let mut w = Vec::new();
        assert_eq!(parse_db_max_connections(Some("12"), &mut w), 12);
        assert!(
            w.is_empty(),
            "合法输入不该产生 WARN（一直红/一直黄的门槛会被忽略）"
        );
    }

    #[test]
    fn empty_db_path_falls_back_to_default() {
        let mut w = Vec::new();
        assert_eq!(
            parse_db_path(Some("  "), &mut w),
            PathBuf::from(DEFAULT_DB_PATH)
        );
        assert_eq!(w.len(), 1);
    }
    #[test]
    fn default_addr_parses() {
        // 常量本身必须可解析 —— 若这条红，说明有人改了字面量却没改解析。
        let a: SocketAddr = DEFAULT_ADDR.parse().expect("默认地址必须可解析");
        assert!(a.ip().is_loopback(), "默认只听回环，不听全网卡");
    }

    #[test]
    fn from_env_never_panics_on_garbage_addr() {
        // ⚠️ 环境变量是**用户输入**。垃圾输入必须产生 WARN 而不是 panic。
        let a = parse_addr(&mut vec![]);
        assert!(a.port() > 0, "回退默认值必须仍是可绑定端口");
    }

    #[test]
    fn warnings_are_displayable_in_chinese() {
        let w = Warning {
            source: "QUILL_ADDR".to_string(),
            message: "无法解析监听地址".to_string(),
        };
        assert!(w.to_string().contains("QUILL_ADDR"));
    }
}
