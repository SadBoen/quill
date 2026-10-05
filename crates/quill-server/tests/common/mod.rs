//! 打真库的共享夹具：**临时 SQLite 文件**，不是内存库。
//!
//! # 为什么必须是文件而不是 `sqlite::memory:`
//!
//! 内存库的 WAL 是 noop（`quill_store::in_memory` 的注释已写明），
//! 而本任务要验的事里至少两件依赖真实文件行为：
//! 1. **重启即失**的修复 —— 换一条桥接（模拟进程重启）后数据仍在；
//! 2. 外键与 `busy_timeout` 在多条连接上的行为。
//!
//! 内存库还会在连接数 > 1 时「每个连接一份独立库」，那是另一种假象。
//!
//! # 为什么不用 `include_str!` 读迁移
//!
//! `scripts/check-boundary-singletons.sh`（G26-3）把 `include!` 的跨 crates
//! 目标判红；`include_str!` 虽不匹配该正则，但本项目已有一条更硬的规矩：
//! **不在编译期跨 crate 拷贝别人的文件**（那会造出第二份真相源）。
//! 因此这里在**运行期**按 `CARGO_MANIFEST_DIR` 读 `quill-store` 的迁移文件，
//! 迁移的唯一真相源仍然是 `crates/quill-store/migrations/0001_init.sql`。
//!
//! # 为什么本模块自带一条自检用例
//!
//! 集成测试的每个二进制各自编译一份本模块。若某条查询助手只在部分文件里
//! 被用到，没用到的那些二进制会报 `dead_code`（而验收要求
//! `clippy --all-targets -- -D warnings` 零警告）。
//! 与其加 `#[allow(dead_code)]`（那会让「没人用」变得不可见），
//! 不如让本模块自己**真的用一遍**：末尾的 `fixture_queries_really_hit_the_database`
//! 既消灭了告警，也顺带证明「助手函数 + 临时库」这套夹具本身没坏 ——
//! 夹具坏了的话，所有用例的「行真的落库了」断言都会变成假绿。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use quill_server::db::{storage_error, DbBridge};

/// 并行用例的目录序号（同一进程内多个用例各拿一个目录）。
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 一个已迁移的临时数据库。
pub struct TestDb {
    bridge: Option<Arc<DbBridge>>,
    dir: PathBuf,
}

impl TestDb {
    /// 建一个新的临时库并执行全部迁移。
    ///
    /// `label` 只用于让目录名可读（失败时一眼看出是哪个用例）。
    pub fn new(label: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "quill-server-test-{label}-{}-{n}",
            std::process::id()
        ));
        // ⚠️ 先删再建：上一次异常退出可能留下同名目录。
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("创建临时目录 {} 失败：{e}", dir.display()));
        let path = dir.join("quill.db");
        let bridge = Arc::new(
            DbBridge::open(path.to_str().expect("临时路径必须是 UTF-8"), 2)
                .unwrap_or_else(|e| panic!("打开临时数据库失败：{e}")),
        );
        let n = bridge
            .migrate(&migration_sql())
            .unwrap_or_else(|e| panic!("执行迁移失败：{e}"));
        assert!(n > 0, "迁移语句数为 0：schema 文件读到了但没执行任何语句");
        let missing = bridge
            .missing_tables()
            .unwrap_or_else(|e| panic!("探测表失败：{e}"));
        assert!(
            missing.is_empty(),
            "迁移后仍缺表 {missing:?}：测试前提不成立，不能继续（否则后面全是假绿）"
        );
        Self {
            bridge: Some(bridge),
            dir,
        }
    }

    /// 取桥接句柄（克隆，调用方通常需要放进 `AppState`）。
    pub fn bridge(&self) -> Arc<DbBridge> {
        Arc::clone(self.bridge.as_ref().expect("TestDb 持有桥接"))
    }

    /// 库文件路径（供「换一条桥接模拟重启」的用例）。
    pub fn path(&self) -> String {
        self.dir.join("quill.db").to_string_lossy().to_string()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // ⚠️ 顺序：先释放桥接（连接池与工作线程在这里 join 并关闭），
        //    再删目录。反过来会在 Windows 上删不掉、且 WAL 残留。
        if let Some(b) = self.bridge.take() {
            drop(b);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 读迁移 SQL（运行期按路径读，理由见模块注释）。
pub fn migration_sql() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quill-store/migrations/0001_init.sql");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取迁移文件 {} 失败：{e}", path.display()))
}

/// 在真库上跑一条查询并取回第一行的某列（整数）。
pub fn scalar_i64(db: &Arc<DbBridge>, sql: &str) -> i64 {
    // ⚠️ 必须转成 `String` 再进闭包：`DbBridge::call` 的闭包是 `'static`
    //    （它要跨线程），借用的 `&str` 逃不出去。
    let sql = sql.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query_scalar(&sql)
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("测试断言查询", e))
        })
    })
    .unwrap_or_else(|e| panic!("断言查询失败：{e}"))
}

/// 在真库上跑一条查询并取回第一行的 `v` 列（文本）。
///
/// ⚠️ 固定列名 `v`：让调用处的 SQL 一眼可读（`SELECT state AS v …`）。
pub fn text_of(db: &Arc<DbBridge>, sql: &str) -> String {
    let sql = sql.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let row = sqlx::query(&sql)
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("测试断言查询", e))?;
            sqlx::Row::try_get::<String, _>(&row, "v").map_err(|e| storage_error("测试断言取列", e))
        })
    })
    .unwrap_or_else(|e| panic!("断言查询失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 夹具自检：**证明这套助手真的打到了真库**，而不是「查了个寂寞」。
    ///
    /// 判据三条：
    /// 1. 新库的两张目标表都是 0 行（说明查的是迁移后的真表，不是内存 map）；
    /// 2. 写进去一行后能按文本列查回来（说明助手不是恒返回默认值）；
    /// 3. `path()` 指得到一个真文件（重启用例的前提）。
    #[test]
    fn fixture_queries_really_hit_the_database() {
        let t = TestDb::new("fixture-selfcheck");
        assert_eq!(
            scalar_i64(&t.bridge(), "SELECT COUNT(*) AS c FROM experts"),
            0,
            "新库 experts 必须是空的"
        );
        assert_eq!(
            scalar_i64(&t.bridge(), "SELECT COUNT(*) AS c FROM task_dispatches"),
            0,
            "新库 task_dispatches 必须是空的"
        );
        sqlx_write(&t.bridge());
        assert_eq!(
            text_of(
                &t.bridge(),
                "SELECT display_name AS v FROM experts WHERE id = 'probe-expert'"
            ),
            "夹具探针",
            "写入的行必须能查回来"
        );
        assert!(
            std::path::Path::new(&t.path()).is_file(),
            "path() 必须指向一个真实库文件：{}",
            t.path()
        );
    }

    fn sqlx_write(db: &Arc<DbBridge>) {
        db.call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO users (id, username, username_norm, display_name, \
                     password_hash, password_salt, password_algo, role, pwd_changed_at, \
                     created_at, updated_at) \
                     VALUES (x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0', 'probe', 'probe', '探针', \
                     zeroblob(32), zeroblob(16), 'pbkdf2-hmac-sha256$i=600000', 'owner', \
                     0, 0, 0)",
                )
                .execute(&pool)
                .await
                .map_err(|e| storage_error("夹具探针写用户", e))?;
                // ⚠️ 直接写一行最小可用的 experts：它没有外键，
                //    因此不需要 users/teams 前提（那正是它适合当探针的原因）。
                sqlx::query(
                    "INSERT INTO experts (id, owner_user_id, display_name, version, description, \
                     role_summary, visibility, tool_policy_json, tags_json, license, \
                     default_enabled, is_builtin, asset_hash, persona_hash, skill_count, \
                     created_at, updated_at, deleted_at) \
                     VALUES ('probe-expert', x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0', '夹具探针', '0.1.0', \
                     '', '', 'user_authored', '{}', '[]', '', 1, 0, zeroblob(32), zeroblob(32), \
                     0, 0, 0, NULL)",
                )
                .execute(&pool)
                .await
                .map_err(|e| storage_error("夹具探针写专家", e))?;
                Ok(())
            })
        })
        .expect("夹具探针写入失败");
    }
}
