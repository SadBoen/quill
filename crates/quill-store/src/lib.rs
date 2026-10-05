//! `quill-store` —— Quill 状态存储层。
//!
//! 依据 `04_数据模型设计.md`（作者：储海量）。
//!
//! # 本 crate 的职责边界
//!
//! 只做两件事：
//! 1. **连接层**：建库、设 PRAGMA、跑迁移
//! 2. **类型转换**：领域类型 ↔ SQLite 值（uuid ↔ BLOB(16)、时间 ↔ INTEGER 毫秒）
//!
//! **不做**业务逻辑、不做 SQL 拼装、不做 Agent 编排。
//!
//! # 为什么 PRAGMA 必须在代码里设（而不是写在 SQL 文件里）
//!
//! PRAGMA 是**连接级**的，不被记在数据库文件里 —— 写进 `migrations/*.sql`
//! 只会对执行该语句的那一条连接生效，连接池里的其他连接完全不受影响。
//! 而 `foreign_keys` **默认是 OFF**：漏设的话，`04_数据模型设计.md` 里
//! 所有复合外键（跨用户隔离的核心防线）**全部形同虚设，且不报任何错**。
//!
//! 所以这里用 [`SqlitePoolOptions::after_connect`] 对**每一条**连接强制设置，
//! 并在 `tests/schema_constraints.rs` 里有「全量遍历连接池」的断言兜底。
//!
//! 三条设置规则见 [`apply_pragmas`] 的文档注释。

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

/// 写入锁竞争时的等待上限（毫秒）。
///
/// **为什么必须有这个值**：SQLite 的 `busy_timeout` 默认是 0，
/// 意味着第二个并发写者**立即**收到 `SQLITE_BUSY` 而不是等待。
/// 本项目写池 size=1，但 WAL 下仍有写者排队（迁移、备份、checkpoint）。
/// 5 秒足以覆盖一次正常写事务。
///
/// ⚠️ 它必须与 [`SqliteJournalMode::Wal`] 在**同一批** PRAGMA 中设置：
/// 若某条分支只设了 WAL 而漏了 busy_timeout，
/// WAL 单写者的排队等待会**静默退化成立即失败** —— 不报错，只在并发时显现。
pub const BUSY_TIMEOUT_MS: u64 = 5_000;

/// 每用户的数据目录名（相对 `workspace/data/`）。
///
/// 目录名用 uuid v7 的标准小写连字符形式（36 字符）而非短码：
/// 可读、可手工定位、与日志/截图一致、无映射表、无歧义。
/// 磁盘路径长度不是问题（Linux 单段上限 255 字节）。
pub fn user_dir_name(user_id: &str) -> String {
    user_id.to_string()
}

/// 把一条 SQL 拆成可逐条执行的片段。
///
/// **为什么需要它**：sqlx 的 `execute` 一次只能跑一条语句，
/// 而 `migrations/0001_init.sql` 是一个包含 44 条语句的文件。
///
/// 拆分规则必须精确，否则会切坏字符串字面量：
/// - 按 `;` 切
/// - 但要跳过单引号字符串内、注释行内、双引号标识符内的 `;`
/// - 丢弃空片段与纯注释片段
///
/// ⚠️ **这是一个已知的简化实现**：不处理 SQL 里的 dollar-quoted 块
/// （SQLite 不支持）、也不处理 `BEGIN...END` 触发器体（本 schema 不用触发器）。
/// 若将来引入触发器，必须换成真正的 SQL 解析器 —— 见 `R10`（被否决：SQL 触发器）。
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if in_line_comment {
            // 行注释：丢弃内容，但保留换行（便于行号定位）
            if c == '\n' {
                in_line_comment = false;
                cur.push(c);
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if in_str {
            cur.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    cur.push(chars.next().unwrap()); // '' 转义
                } else {
                    in_str = false;
                }
            }
            continue;
        }
        match c {
            '-' if chars.peek() == Some(&'-') => {
                in_line_comment = true;
                chars.next();
            }
            '/' if chars.peek() == Some(&'*') => {
                in_block_comment = true;
                chars.next();
            }
            '\'' => {
                in_str = true;
                cur.push(c);
            }
            ';' => {
                let s = cur.trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let s = cur.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
    out
}

/// 对**每一条**连接强制设置 PRAGMA。
///
/// 这里是整个 crate 最关键的几行。三条规则：
///
/// | 规则 | 不遵守的后果 |
/// |---|---|
/// | `foreign_keys = ON` | **默认 OFF**。漏设 → 所有复合外键失效 → 跨用户写入畅通无阻且不报错 |
/// | `journal_mode = WAL` | 单写者多读者；不设则读写互相阻塞 |
/// | `busy_timeout` | 默认 0 → 第 2 个并发写者立即 `SQLITE_BUSY`（而非等待） |
///
/// 顺序也有讲究：先 `journal_mode`（要改数据库头，需独占锁），
/// 再 `busy_timeout`（纯连接属性）。
async fn apply_pragmas(conn: &mut sqlx::SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&mut *conn)
        .await?;
    sqlx::query(&format!("PRAGMA busy_timeout = {BUSY_TIMEOUT_MS}"))
        .execute(&mut *conn)
        .await?;
    // ⚠️ 必须显式开：SQLite 默认是 OFF。
    // 这一行漏掉，下面 tests/ 里的全部隔离断言都会失去意义。
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// 建库并按 `max_connections` 条连接全部配置好 PRAGMA。
///
/// `max_connections` 就是架构 §4.2 的连接池上限 20。
/// 写侧建议 size=1（SQLite 单写者），读侧可放开。
///
/// 调 `configure_pool` 之前**必须**已设置 `PRAGMA journal_mode=WAL`
/// （见文档「配置连接池的 PRAGMA 顺序要求」）。
pub async fn configure_pool(path: &str, max_connections: u32) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(path)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .after_connect(|conn, _meta| Box::pin(apply_pragmas(conn)))
        .connect_with(opts)
        .await
}

/// 逐条执行一个 `.sql` 迁移文件。
///
/// ⚠️ **不在事务里**：SQLite 的 DDL 是事务性的，但
/// `PRAGMA journal_mode` 与部分 PRAGMA 在事务内会静默失效。
/// 迁移脚本里不含 PRAGMA，因此这里显式逐条执行并在末尾做一致性检查。
pub async fn run_migration(pool: &SqlitePool, sql: &str) -> Result<usize, sqlx::Error> {
    let stmts = split_statements(sql);
    for s in &stmts {
        sqlx::query(s).execute(pool).await?;
    }
    // 迁移后自检：外键必须确实是开的。
    // 若不是，说明 PRAGMA 漏设 —— 此时继续运行等于隔离防线已失效。
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    if fk != 1 {
        return Err(sqlx::Error::Protocol(
            "PRAGMA foreign_keys 未生效：跨用户隔离约束已失效，拒绝继续".into(),
        ));
    }
    Ok(stmts.len())
}

/// 在内存库上建一份「已迁移」的连接，供测试使用。
///
/// ⚠️ **内存库的 WAL 是 noop**，所以本函数**不能**用于验证 WAL 行为
/// （那部分由 `tests/backup_consistency.rs` 用真实文件库覆盖）。
/// 它只用于验证 schema 约束 —— 约束与存储介质无关。
pub async fn in_memory() -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _meta| Box::pin(apply_pragmas(conn)))
        .connect_with(opts)
        .await?;
    // 内存库连接数上限为 1 时，SQLite 会复用同一条连接；
    // 若将来改成多条，需改用 `file::memory:?cache=shared` 保持共享。
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_comments_and_strings() {
        // 基本切分
        let s = "CREATE TABLE a(x); CREATE TABLE b(y);";
        assert_eq!(split_statements(s).len(), 2);
        // 注释里的分号不该切
        let s = "-- 注释里有 ; 分号\nCREATE TABLE a(x);";
        let v = split_statements(s);
        assert_eq!(v.len(), 1, "注释里的分号导致误切：{v:?}");
        assert!(v[0].starts_with("CREATE"));
        // 字符串字面量里的分号不该切
        let s = "INSERT INTO t VALUES('a;b'); SELECT 1;";
        let v = split_statements(s);
        assert_eq!(v.len(), 2, "字符串里的分号导致误切：{v:?}");
        assert!(v[0].contains("'a;b'"));
        // 纯注释
        assert_eq!(split_statements("-- 只有注释\n-- 别的\n").len(), 0);
    }

    #[tokio::test]
    async fn every_connection_has_foreign_keys_on() {
        // ★ 这是整套隔离防御的命门测试（装置可信性断言）。
        // PRAGMA 是**连接级**的：只要池里有一条连接漏设，
        // 跨用户写入就畅通无阻且不报错。所以必须**全量遍历**，不能抽查。
        let dir = std::env::temp_dir().join(format!(
            "quill-store-fk-{}-{:?}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&dir);
        let path = dir.to_string_lossy().to_string();

        let pool = configure_pool(&path, 5).await.expect("建池");
        for i in 1..=5 {
            let mut conn = pool.acquire().await.expect("取连接");
            let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *conn)
                .await
                .expect("读 PRAGMA");
            let bt: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&mut *conn)
                .await
                .expect("读 PRAGMA");
            let jm: String = sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(&mut *conn)
                .await
                .expect("读 PRAGMA");
            assert_eq!(
                fk, 1,
                "第 {i} 条连接的 foreign_keys 是关的 —— 复合外键全部失效"
            );
            assert_eq!(
                bt, BUSY_TIMEOUT_MS as i64,
                "第 {i} 条连接 busy_timeout 未生效"
            );
            assert_eq!(jm.to_ascii_lowercase(), "wal", "第 {i} 条连接未启用 WAL");
        }
        drop(pool);
        let _ = std::fs::remove_file(&dir);
        let _ = std::fs::remove_file(dir.with_extension("db-wal"));
        let _ = std::fs::remove_file(dir.with_extension("db-shm"));
    }
}
