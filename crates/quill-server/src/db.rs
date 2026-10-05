
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::runtime::Runtime;

use quill_agent::AgentError;

pub const DB_CALL_TIMEOUT: Duration = Duration::from_secs(30);

const WORKER_THREAD_NAME: &str = "quill-db";

pub const REQUIRED_TABLES: [&str; 2] = ["experts", "task_dispatches"];

type Job = Box<dyn FnOnce(SqlitePool, &mut Runtime) + Send + 'static>;

#[derive(Clone)]
pub struct DbBridge {
    jobs: mpsc::Sender<Job>,
    worker: Arc<WorkerHandle>,
    path: String,
}

struct WorkerHandle {
    handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,

    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {

        if let Ok(mut slot) = self.handle.lock() {
            if let Some(h) = slot.take() {
                let _ = h.join();
            }
        }
    }
}

impl fmt::Debug for DbBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        f.debug_struct("DbBridge")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl DbBridge {

    pub fn open(path: &str, max_connections: u32) -> Result<Self, AgentError> {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_path = path.to_string();
        let thread_alive = Arc::clone(&alive);

        let worker = std::thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_string())
            .spawn(move || {
                let mut rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("无法创建存储线程的异步运行时：{e}")));
                        return;
                    }
                };
                let pool = match rt
                    .block_on(quill_store::configure_pool(&worker_path, max_connections))
                {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("无法打开数据库 {worker_path}：{e}")));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {

                    return;
                }
                thread_alive.store(true, std::sync::atomic::Ordering::SeqCst);

                while let Ok(job) = job_rx.recv() {
                    job(pool.clone(), &mut rt);
                }
                thread_alive.store(false, std::sync::atomic::Ordering::SeqCst);
                drop(pool);
            })
            .map_err(|e| AgentError::Storage {
                detail: format!(
                    "无法创建存储线程 {WORKER_THREAD_NAME}：{e}。\
                         下一步：检查系统线程数上限（执行 `ulimit -u`），\
                         或用 `QUILL_DB_PATH` 换一个可写的数据库路径后重启服务。"
                ),
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                jobs: job_tx,
                worker: Arc::new(WorkerHandle {
                    handle: std::sync::Mutex::new(Some(worker)),
                    alive,
                }),
                path: path.to_string(),
            }),
            Ok(Err(msg)) => {

                let _ = worker.join();
                Err(AgentError::Storage { detail: msg })
            }
            Err(_) => {
                let _ = worker.join();
                Err(AgentError::Storage {
                    detail: format!(
                        "存储线程在报告就绪前就退出了（数据库 {path}）。\
                         下一步：确认父目录存在且可写（执行 `ls -ld $(dirname {path})`），\
                         然后用 `quill doctor --section=db` 查看完整诊断。"
                    ),
                })
            }
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn is_alive(&self) -> bool {
        self.worker.alive.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn call<T, F>(&self, f: F) -> Result<T, AgentError>
    where
        T: Send + 'static,
        F: FnOnce(SqlitePool, &mut Runtime) -> BoxFuture<'static, Result<T, AgentError>>
            + Send
            + 'static,
    {
        let (tx, rx) = mpsc::sync_channel::<Result<T, AgentError>>(1);
        let job: Job = Box::new(move |pool, rt| {
            let out = f(pool, rt);

            let _ = tx.send(rt.block_on(out));
        });
        if self.jobs.send(job).is_err() {
            return Err(AgentError::Storage {
                detail: "存储线程已退出，无法执行数据库操作（服务正在关闭或存储已失效）。\
                         下一步：执行 `quill doctor --section=db` 查看存储状态，确认后重启服务。"
                    .to_string(),
            });
        }
        match rx.recv_timeout(DB_CALL_TIMEOUT) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(AgentError::Storage {
                detail: format!(
                    "数据库操作超过 {} 秒仍未返回，已放弃本次调用。\
                     这通常意味着数据库文件被另一个进程长时间独占。\
                     下一步：执行 `lsof data/quill.db` 找出占用进程，\
                     确认无冲突后重试；并用 `quill doctor --section=db` 查看完整诊断。",
                    DB_CALL_TIMEOUT.as_secs()
                ),
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AgentError::Storage {
                detail: "存储线程在执行本次操作时异常退出。\
                         下一步：查看服务端 stderr 中 [quill-db] 相关日志，\
                         并用 `quill doctor --section=db` 查看完整诊断。"
                    .to_string(),
            }),
        }
    }

    pub fn migrate(&self, sql: &str) -> Result<usize, AgentError> {
        let sql = sql.to_string();
        self.call(move |pool, _rt| {
            Box::pin(async move {
                quill_store::run_migration(&pool, &sql)
                    .await
                    .map_err(|e| storage_error("执行数据库迁移", e))
            })
        })
    }

    pub fn missing_tables(&self) -> Result<Vec<String>, AgentError> {
        self.call(|pool, _rt| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN (?, ?)",
                )
                .bind(REQUIRED_TABLES[0])
                .bind(REQUIRED_TABLES[1])
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("探测数据库表结构", e))?;
                let mut present: Vec<String> = Vec::with_capacity(rows.len());
                for r in &rows {
                    present.push(col!(r, String, "name", "探测数据库表结构"));
                }
                Ok(REQUIRED_TABLES
                    .iter()
                    .filter(|t| !present.iter().any(|p| p == *t))
                    .map(|t| (*t).to_string())
                    .collect::<Vec<String>>())
            })
        })
    }

    pub fn schema_ready(&self) -> Result<bool, AgentError> {
        Ok(self.missing_tables()?.is_empty())
    }
}

impl Drop for DbBridge {
    fn drop(&mut self) {

    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub fn storage_error(op: &str, e: sqlx::Error) -> AgentError {
    AgentError::Storage {
        detail: format!(
            "{op}失败：{e}。\
             下一步：执行 `quill doctor --section=db` 打印本错误的完整诊断与修复动作。"
        ),
    }
}

pub fn invariant_broken(detail: impl Into<String>) -> AgentError {
    AgentError::InvariantBroken {
        detail: detail.into(),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

macro_rules! col {
    ($row:expr, $ty:ty, $name:literal, $op:expr) => {
        sqlx::Row::try_get($row, $name).map_err(|e| $crate::db::storage_error($op, e))?
    };
}
pub(crate) use col;

fn fnv128(seed: u64, parts: &[&[u8]]) -> [u8; 16] {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = seed;
    for p in parts {
        for b in *p {
            h ^= u64::from(*b);
            h = h.wrapping_mul(FNV_PRIME);
        }

        h ^= 0xff;
        h = h.wrapping_mul(FNV_PRIME);
    }
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h.to_be_bytes());
    out[8..].copy_from_slice(&h.rotate_left(32).wrapping_add(seed).to_be_bytes());
    out
}

pub fn digest32(label: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    all.push(label.as_bytes());
    all.extend_from_slice(parts);
    let a = fnv128(0x243f_6a88_85a3_08d3, &all);
    let b = fnv128(0x1319_8a2e_0370_7344, &all);
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&a);
    out[16..].copy_from_slice(&b);
    out
}

pub fn digest16(label: &str, parts: &[&[u8]]) -> [u8; 16] {
    let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    all.push(label.as_bytes());
    all.extend_from_slice(parts);
    fnv128(0x9e37_79b9_7f4a_7c15, &all)
}

pub fn user_id_from_blob(raw: &[u8]) -> Option<quill_adapters::UserId> {
    let arr: [u8; 16] = raw.try_into().ok()?;
    Some(quill_adapters::UserId::from_bytes(arr))
}

pub fn blob_of(id: &quill_adapters::UserId) -> Vec<u8> {
    id.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_label_sensitive() {
        let a = digest32("asset", &[b"cost-analyst"]);
        let b = digest32("asset", &[b"cost-analyst"]);
        let c = digest32("persona", &[b"cost-analyst"]);
        let d = digest32("asset", &[b"cost-analyst-2"]);
        assert_eq!(a, b, "同输入必须同摘要（否则 upsert 每次都写新摘要）");
        assert_ne!(a, c, "标签不同必须不同摘要");
        assert_ne!(a, d, "输入不同必须不同摘要");
    }

    #[test]
    fn digest16_distinguishes_segment_boundaries() {

        let a = digest16("dispatch-id", &[b"ab", b"c"]);
        let b = digest16("dispatch-id", &[b"a", b"bc"]);
        assert_ne!(a, b, "分段边界不同必须产出不同标识");
    }

    #[test]
    fn blob_roundtrip() {
        let id = quill_adapters::UserId::from_bytes([7u8; 16]);
        assert_eq!(user_id_from_blob(&blob_of(&id)), Some(id));
        assert_eq!(user_id_from_blob(&[0u8; 15]), None, "长度不对必须判 None");
    }
}
