use std::net::SocketAddr;
use std::path::PathBuf;

pub const DEFAULT_ADDR: &str = "127.0.0.1:8848";

pub const DEFAULT_WEB_DIR: &str = "ui/web/dist";

pub const DEFAULT_DB_PATH: &str = "data/quill.db";

pub const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub source: String,

    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,

    pub web_dir: PathBuf,

    pub enable_selftest: bool,

    pub db_path: PathBuf,

    pub db_max_connections: u32,

    pub warnings: Vec<Warning>,
}

impl Config {
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

    pub fn ui_assets_available(&self) -> bool {
        self.web_dir.is_dir()
    }
}

fn parse_addr(warnings: &mut Vec<Warning>) -> SocketAddr {
    match std::env::var("QUILL_ADDR") {
        Err(_) => DEFAULT_ADDR
            .parse()
            .expect("DEFAULT_ADDR 是本文件内的字面量常量，非用户输入"),
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

fn parse_db_max_connections(raw: Option<&str>, warnings: &mut Vec<Warning>) -> u32 {
    match raw {
        None => DEFAULT_DB_MAX_CONNECTIONS,
        Some(raw) => match raw.trim().parse::<u32>() {
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
        let a: SocketAddr = DEFAULT_ADDR.parse().expect("默认地址必须可解析");
        assert!(a.ip().is_loopback(), "默认只听回环，不听全网卡");
    }

    #[test]
    fn from_env_never_panics_on_garbage_addr() {
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
