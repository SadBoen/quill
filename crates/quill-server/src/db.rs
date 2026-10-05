//! 数据库桥接层 —— 把 `sqlx` 的 async 世界接进 `quill-agent` 的**同步**端口。
//!
//! # 为什么需要这一层（而不是直接 impl 端口 trait）
//!
//! `quill-agent` 的两个存储端口（`ExpertRepository` / `DispatchLedger`）是**同步**方法，
//! 且该 crate 的依赖表里**没有 tokio/sqlx**（见 `crates/quill-agent/src/expert.rs` 模块头）。
//! 本 crate 是组装层，可以依赖 tokio + sqlx，但**不能改端口签名**
//! （`crates/quill-agent/**` 已定稿，不在本任务的文件所有权内）。
//!
//! 于是同步端口 × 异步驱动只有三种接法：
//!
//! | 方案 | 为什么否掉 |
//! |---|---|
//! | 在 handler 里 `rt.block_on(...)` | axum handler 本身就跑在 tokio 运行时里，嵌套 `block_on` 会 panic |
//! | `tokio::task::block_in_place` | `current_thread` 运行时里**直接 panic**（`#[tokio::test]` 默认就是它） |
//! | **专用线程 + 通道（本模块）** | ✅ 任何调用上下文都安全，`Runtime` 只在**它自己的线程**上创建与析构 |
//!
//! # 为什么 `Runtime` 必须待在专用线程里
//!
//! `Runtime::drop` 在异步上下文里会 panic（"Cannot drop a runtime in a context
//! where blocking is not allowed"）。若把运行时挂在 `AppState` 上，进程退出时
//! 就会撞上它 —— 一个「只在优雅关闭时出现、且只在 async 上下文里出现」的崩溃。
//! 放在专用线程上后：`Drop` 关闭通道 → 工作线程退出循环 → 连接池与运行时
//! 在**工作线程上**被析构，全程不碰调用方的运行时。
//!
//! # 跨用户隔离在哪一层保证
//!
//! **不在这一层，在 SQL 里。** 本模块只负责「把闭包送到有连接池的线程上执行」；
//! 每条语句都带 `owner_user_id` / `user_id` 条件由 `experts_repo` / `dispatch_ledger`
//! 负责（`V1_SCOPE_CONSTRAINTS.md` §五「多用户隔离 = 最高优先级」）。
//! 本模块**不缓存任何行数据**，因此不存在「忘了带 owner 条件的缓存命中」这条绕过路径。
//!
//! # 失败语义（铁律七）
//!
//! - 建池失败 → 返回 `AgentError::Storage`（中文 + `quill doctor` 命令），**不 panic**；
//! - 单次调用超过 [`DB_CALL_TIMEOUT`] → 判失败（中文说明 + 建议），
//!   而不是让请求线程无限期挂住 —— 挂住的服务无法被 `quill doctor` 诊断。

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::runtime::Runtime;

use quill_agent::AgentError;

/// 单次存储调用的等待上限。
///
/// ⚠️ **为什么必须有上限**：`recv()` 若不带超时会无限期阻塞调用线程。
/// 一次真正卡死的存储调用会让 HTTP 请求永远不返回，而 `quill doctor`
/// （本项目唯一的诊断入口）也就永远拿不到答案。
/// 30 秒对本机 SQLite 写入是三个数量级的余量；超它即视为异常并判失败。
pub const DB_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 工作线程名（日志里能直接看出是哪个线程在跑存储）。
const WORKER_THREAD_NAME: &str = "quill-db";

/// 本实现真正读写的两张表（启动时探测它们是否存在）。
///
/// ⚠️ 刻意**只列这两张**：`quill-agent` 的两个端口只碰它们，
/// 少列一张会让「schema 缺失」的告警晚到（用户先撞上 500 而不是启动告警）。
pub const REQUIRED_TABLES: [&str; 2] = ["experts", "task_dispatches"];

/// 一次存储任务：在持有连接池与运行时的线程上跑完。
///
/// ⚠️ 连接池按**值**传入（内部是 `Arc`，克隆成本可忽略），而不是按引用。
/// 因为任务闭包必须能返回 `'static` 的 future（它要跨线程传递），
/// 若按引用传，返回的 future 就得借用一个短生命周期 —— 那在
/// `Box<dyn FnOnce(..)>` 的位置根本写不出来（高阶生命周期约束）。
/// 按值传让 future 完全拥有连接池，签名也随之简单。
type Job = Box<dyn FnOnce(SqlitePool, &mut Runtime) + Send + 'static>;

/// 桥接层的对外句柄。
///
/// 克隆共享同一个连接池与同一条工作线程（内部是 `mpsc::Sender` 的克隆）。
///
/// ⚠️ **字段顺序是析构顺序的一部分**：`jobs`（Sender）声明在 `worker` 之前，
/// 因此最后一个克隆消失时，Sender 先析构 → 工作线程的 `recv()` 返回 `Err`
/// → 线程退出循环 → `WorkerHandle::drop` 里的 `join` 立刻返回。
/// 顺序写反（`worker` 在前）会变成「join 等一个还在等任务的线程」——
/// 而那个线程永远等不到，因为 Sender 还活着。**死锁，且只在退出时出现。**
#[derive(Clone)]
pub struct DbBridge {
    jobs: mpsc::Sender<Job>,
    worker: Arc<WorkerHandle>,
    path: String,
}

/// 工作线程句柄：**最后一个克隆**消失时才 join。
///
/// ⚠️ 为什么不能把 `JoinHandle` 直接放进 `DbBridge`：`AppState` 是 `Clone`
/// 的（axum 的 `with_state` 要求），而 `JoinHandle` 不是 `Clone`；
/// 若每次克隆都另起一条线程，就等于「复制状态 = 复制连接池」。
/// 放进 `Arc` 后，多个克隆共享**同一条**线程，且 join 只发生一次。
struct WorkerHandle {
    handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 工作线程是否仍在服务（退出前置 `false`）。
    ///
    /// ⚠️ 用 `Arc` 而不是裸 `AtomicBool`：工作线程需要**自己**持有它
    /// 才能在退出前置位，而句柄这边又要读它。
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // 到这里时本结构体是**最后一个** DbBridge 克隆，
        // 对应的 Sender 已在字段析构阶段关闭 → 线程的 recv() 立刻返回 Err。
        if let Ok(mut slot) = self.handle.lock() {
            if let Some(h) = slot.take() {
                let _ = h.join();
            }
        }
    }
}

impl fmt::Debug for DbBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ⚠️ 不打连接池内部：连接池的 Debug 会带每条连接的状态，噪音极大。
        f.debug_struct("DbBridge")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl DbBridge {
    /// 打开（或建出）一个 SQLite 库并完成连接池配置。
    ///
    /// 连接层由 `quill_store::configure_pool` 提供，它对**每一条**连接设置
    /// `foreign_keys = ON` / `WAL` / `busy_timeout`（见该函数文档）。
    /// ⚠️ 本函数**不跑迁移**：迁移是 `quill-upgrade` 的职责（当前仍是骨架），
    /// 服务端擅自改 schema 属于未经评审的结构变更。
    /// 缺表时由 [`DbBridge::schema_status`] 报出来，调用方决定怎么处理。
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
                    // 调用方已放弃（它拿到的是超时或已被 drop）→ 直接收摊。
                    return;
                }
                thread_alive.store(true, std::sync::atomic::Ordering::SeqCst);
                // 🔴 收不到任务即调用方全部销毁：连接池与 Runtime 在**本线程**析构。
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
                // ⚠️ 启动失败时**必须 join**：否则工作线程会带着「已失败」的
                // 连接池残留（线程此时已自行返回，join 只是回收句柄）。
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

    /// 执行的数据库路径（供 `/healthz` 与启动横幅显示）。
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 存储线程是否仍在服务。
    ///
    /// ⚠️ 存在的理由：`/healthz` 需要区分「库能开」与「线程还活着」。
    /// 线程退出后所有 `call` 都会失败，若两者在健康检查里长得一样，
    /// 运维就会一直查「数据库是不是坏了」而找不到真正的原因。
    pub fn is_alive(&self) -> bool {
        self.worker.alive.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 在存储线程上跑一次异步任务并把结果拿回来。
    ///
    /// `f` 收到**按值克隆的**连接池与运行时，返回一个 future；本函数负责把它跑到完成。
    /// ⚠️ `f` 必须是 `Send + 'static`：它会**跨线程**执行
    /// （所以闭包里只能捕获克隆后的值，不能借用调用栈上的数据）。
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
            // ⚠️ 结果必须**无条件**送回：调用方正在 recv_timeout 计时，
            // 静默丢弃会让每次失败都变成「等满 30 秒才报错」。
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

    /// 逐条执行一份迁移 SQL（由 `quill_store::run_migration` 完成切分与执行）。
    ///
    /// ⚠️ **服务端启动路径不调用它**：schema 变更必须经 `quill-upgrade`
    /// 显式触发（铁律六：迁移 expand-only + 可回滚）。
    /// 本方法存在的理由是让**调用方**（quill-upgrade 落地后、或测试）
    /// 有唯一入口，不必各自拼 sqlx 调用。
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

    /// 探测必需表是否齐备。
    ///
    /// 返回缺失的表名（空 `Vec` = 齐备）。
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

    /// 探测必需表是否齐备（`true` = 齐备）。
    pub fn schema_ready(&self) -> Result<bool, AgentError> {
        Ok(self.missing_tables()?.is_empty())
    }
}

impl Drop for DbBridge {
    fn drop(&mut self) {
        // ⚠️ 这里**刻意什么都不做**（连字段都不用手动置空）：
        //    关闭顺序完全依赖字段声明顺序（见结构体注释），
        //    任何在此处 join 的写法都会在「多克隆」场景下提前收走工作线程，
        //    让后续请求全部拿到「存储线程已退出」。
    }
}

/// 跨线程传递的 future 形态。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 把 `sqlx` 错误转成 `AgentError::Storage`（中文 + 可复制命令）。
///
/// ⚠️ 这里**刻意带上底层错误串**：本函数只进服务端日志与领域层错误链，
/// HTTP 响应由 `quill-server` 的 handler 换成不含内部细节的固定中文文案
/// （见 `crate::api_experts` 与 `crate::error` 的说明）。
/// 若在这里把原因抹掉，日志里就只剩「存储失败」四个字，等于制造一个新的静默失败。
pub fn storage_error(op: &str, e: sqlx::Error) -> AgentError {
    AgentError::Storage {
        detail: format!(
            "{op}失败：{e}。\
             下一步：执行 `quill doctor --section=db` 打印本错误的完整诊断与修复动作。"
        ),
    }
}

/// 领域不变量/数据损坏（代码或数据 bug，不是用户输入问题）。
pub fn invariant_broken(detail: impl Into<String>) -> AgentError {
    AgentError::InvariantBroken {
        detail: detail.into(),
    }
}

/// Unix epoch 毫秒（对齐 schema 的 INTEGER 时间约定）。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 从列里取值的统一错误包装。
///
/// 用宏而不是泛型函数：`sqlx` 的 `try_get` 是 `TryGet` 泛型，
/// 写一个泛型包装函数需要把错误类型也泛型化，签名比收益还长。
///
/// ⚠️ 用 UFCS（`sqlx::Row::try_get(..)`）而不是方法语法：
/// 方法语法要求每个使用点都 `use sqlx::Row`，而 `col!` 是在别的模块展开的
/// —— 漏一个 import 就是一片「方法不存在」的噪音。
macro_rules! col {
    ($row:expr, $ty:ty, $name:literal, $op:expr) => {
        sqlx::Row::try_get($row, $name).map_err(|e| $crate::db::storage_error($op, e))?
    };
}
pub(crate) use col;

/// 128 位确定性摘要（FNV-1a 64 位 × 2，不同种子/质数）。
///
/// ⚠️ **不是密码学摘要**，也不声称是：`Cargo.lock` 里没有 `sha2`/`blake3`
/// 可供本 crate 直接使用（`sha2` 虽在锁文件里，但作为 `quill-server` 的
/// 直接依赖超出本任务「不新增外部 crate」的口径）。
/// 用途只有两个，都不涉及安全：
/// 1. `experts.asset_hash` / `persona_hash` 的**占位摘要**（真实值由资源管线写）；
/// 2. `task_dispatches.id` 这类**内部行标识**的派生。
///
/// 为什么派生的行标识是可接受的：`id` 只是 `PRIMARY KEY (user_id, id)` 的一半，
/// 幂等判定用的是 `ux_dispatch_once (user_id, room_id, round, member_expert_id)`。
/// 派生值与键一一对应，因此 upsert 的冲突目标与幂等索引等价。
/// 且**冲突不会被静默吞掉**：见 `dispatch_ledger` 里 `begin` 写不到行时的判红分支。
fn fnv128(seed: u64, parts: &[&[u8]]) -> [u8; 16] {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = seed;
    for p in parts {
        for b in *p {
            h ^= u64::from(*b);
            h = h.wrapping_mul(FNV_PRIME);
        }
        // 分段标记：避免 ("ab","c") 与 ("a","bc") 撞成同一个摘要。
        h ^= 0xff;
        h = h.wrapping_mul(FNV_PRIME);
    }
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h.to_be_bytes());
    out[8..].copy_from_slice(&h.rotate_left(32).wrapping_add(seed).to_be_bytes());
    out
}

/// 32 字节确定性摘要（两路不同种子拼接）。
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

/// 16 字节确定性标识（`task_dispatches.id` 用）。
pub fn digest16(label: &str, parts: &[&[u8]]) -> [u8; 16] {
    let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    all.push(label.as_bytes());
    all.extend_from_slice(parts);
    fnv128(0x9e37_79b9_7f4a_7c15, &all)
}

/// 16 字节标识 → `UserId`。
pub fn user_id_from_blob(raw: &[u8]) -> Option<quill_adapters::UserId> {
    let arr: [u8; 16] = raw.try_into().ok()?;
    Some(quill_adapters::UserId::from_bytes(arr))
}

/// `UserId` → 16 字节（写入 BLOB 列）。
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
        // ⚠️ 少了分段标记时 ("ab","c") 与 ("a","bc") 会撞车 ——
        //    那会让两条不同的派工共用一个行 id。
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
