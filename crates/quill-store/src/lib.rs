use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

pub const BUSY_TIMEOUT_MS: u64 = 5_000;

pub fn user_dir_name(user_id: &str) -> String {
    user_id.to_string()
}

fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if in_line_comment {
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
                    cur.push(chars.next().unwrap());
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

async fn apply_pragmas(conn: &mut sqlx::SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&mut *conn)
        .await?;
    sqlx::query(&format!("PRAGMA busy_timeout = {BUSY_TIMEOUT_MS}"))
        .execute(&mut *conn)
        .await?;

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

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

pub async fn run_migration(pool: &SqlitePool, sql: &str) -> Result<usize, sqlx::Error> {
    let stmts = split_statements(sql);
    for s in &stmts {
        sqlx::query(s).execute(pool).await?;
    }

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

pub async fn in_memory() -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _meta| Box::pin(apply_pragmas(conn)))
        .connect_with(opts)
        .await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_comments_and_strings() {
        let s = "CREATE TABLE a(x); CREATE TABLE b(y);";
        assert_eq!(split_statements(s).len(), 2);

        let s = "-- 注释里有 ; 分号\nCREATE TABLE a(x);";
        let v = split_statements(s);
        assert_eq!(v.len(), 1, "注释里的分号导致误切：{v:?}");
        assert!(v[0].starts_with("CREATE"));

        let s = "INSERT INTO t VALUES('a;b'); SELECT 1;";
        let v = split_statements(s);
        assert_eq!(v.len(), 2, "字符串里的分号导致误切：{v:?}");
        assert!(v[0].contains("'a;b'"));

        assert_eq!(split_statements("-- 只有注释\n-- 别的\n").len(), 0);
    }

    #[tokio::test]
    async fn every_connection_has_foreign_keys_on() {
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
