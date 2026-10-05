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

pub const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
pub const MIGRATION_0002: &str = include_str!("../migrations/0002_admin_config.sql");
pub const MIGRATION_0003: &str = include_str!("../migrations/0003_llm_providers.sql");
pub const MIGRATION_0004: &str = include_str!("../migrations/0004_expert_persona.sql");
pub const MIGRATION_0005: &str = include_str!("../migrations/0005_expert_source_template.sql");
pub const MIGRATION_0006: &str = include_str!("../migrations/0006_teams.sql");
pub const MIGRATION_0007: &str = include_str!("../migrations/0007_mcp_transport_alignment.sql");

pub const MIGRATIONS_TABLES: &[&str] = &[
    "schema_version",
    "users",
    "sessions_auth",
    "invites",
    "experts",
    "mcp_servers",
    "skills",
    "plugins",
    "wiki_index",
    "sessions",
    "teams",
    "team_members",
    "messages",
    "task_dispatches",
    "admin_config",
    "llm_providers",
];

#[derive(Debug)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: MIGRATION_0001,
    },
    Migration {
        version: 2,
        name: "0002_admin_config",
        sql: MIGRATION_0002,
    },
    Migration {
        version: 3,
        name: "0003_llm_providers",
        sql: MIGRATION_0003,
    },
    Migration {
        version: 4,
        name: "0004_expert_persona",
        sql: MIGRATION_0004,
    },
    Migration {
        version: 5,
        name: "0005_expert_source_template",
        sql: MIGRATION_0005,
    },
    Migration {
        version: 6,
        name: "0006_teams",
        sql: MIGRATION_0006,
    },
    Migration {
        version: 7,
        name: "0007_mcp_transport_alignment",
        sql: MIGRATION_0007,
    },
];

fn checksum(sql: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(sql.as_bytes());
    d[..16].to_vec()
}

async fn ledger_exists(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
    )
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub applied: Vec<i64>,
    pub already_current: Vec<i64>,
}

impl MigrationReport {
    pub fn changed(&self) -> bool {
        !self.applied.is_empty()
    }
}

#[derive(Debug)]
pub enum MigrateError {
    Ledger(sqlx::Error),
    Drift { version: i64, name: String },
    Apply { version: i64, name: String, source: sqlx::Error },
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ledger(e) => write!(f, "读写迁移台账失败：{e}"),
            Self::Drift { version, name } => write!(
                f,
                "迁移 {version}（{name}）的内容与已应用的记录不一致：\
                 该迁移文件在首次应用后被改动过。\
                 下一步：确认改动是否有意；数据库结构可能与代码预期不符，\
                 请改用新建库或补一条新的迁移，不要就地改旧迁移。"
            ),
            Self::Apply { version, name, source } => {
                write!(f, "应用迁移 {version}（{name}）失败：{source}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

pub async fn migrate(pool: &SqlitePool) -> Result<MigrationReport, MigrateError> {
    let has_ledger = ledger_exists(pool).await.map_err(MigrateError::Ledger)?;

    let mut report = MigrationReport {
        applied: Vec::new(),
        already_current: Vec::new(),
    };

    for m in MIGRATIONS {
        let sum = checksum(m.sql);
        let prior: Option<Vec<u8>> = if has_ledger {
            sqlx::query_scalar("SELECT checksum FROM schema_version WHERE version = ?")
                .bind(m.version)
                .fetch_optional(pool)
                .await
                .map_err(MigrateError::Ledger)?
        } else {
            None
        };

        if let Some(prev) = prior {
            if prev != sum {
                return Err(MigrateError::Drift {
                    version: m.version,
                    name: m.name.to_string(),
                });
            }
            report.already_current.push(m.version);
            continue;
        }

        let started = std::time::Instant::now();
        run_migration(pool, m.sql).await.map_err(|source| MigrateError::Apply {
            version: m.version,
            name: m.name.to_string(),
            source,
        })?;
        let exec_ms = started.elapsed().as_millis() as i64;

        // 0001 自己建出 schema_version，所以这里必然已经存在该表。
        sqlx::query(
            "INSERT INTO schema_version(version,name,checksum,applied_at,exec_ms,app_version,note)
             VALUES(?,?,?,?,?,?,'')",
        )
        .bind(m.version)
        .bind(m.name)
        .bind(&sum)
        .bind(now_ms())
        .bind(exec_ms)
        .bind(env!("CARGO_PKG_VERSION"))
        .execute(pool)
        .await
        .map_err(MigrateError::Ledger)?;

        report.applied.push(m.version);
    }

    Ok(report)
}

pub async fn schema_is_current(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    if !ledger_exists(pool).await? {
        return Ok(false);
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM schema_version")
        .fetch_one(pool)
        .await?;
    Ok(n as i64 == MIGRATIONS.len() as i64)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

    #[tokio::test]
    async fn migrate_applies_once_then_is_a_no_op() {
        let pool = in_memory().await.expect("内存库");

        // 期望值从 MIGRATIONS 推导，不要手写版本号列表 ——
        // 加一条迁移时手写的那份一定会忘改，然后这个测试变成噪音。
        // 台账里存的是 i64，所以这里转一下。
        let all: Vec<i64> = MIGRATIONS.iter().map(|m| m.version as i64).collect();

        let first = migrate(&pool).await.expect("首次迁移");
        assert_eq!(first.applied, all, "首次必须应用全部迁移");
        assert!(first.changed());

        let second = migrate(&pool).await.expect("二次迁移");
        assert!(second.applied.is_empty(), "二次不得重复应用");
        assert_eq!(second.already_current, all);
        assert!(!second.changed());

        assert!(schema_is_current(&pool).await.expect("查台账"));
    }

    #[tokio::test]
    async fn migrate_creates_every_contract_table_and_records_the_ledger() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        for t in super::MIGRATIONS_TABLES {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(t)
            .fetch_one(&pool)
            .await
            .expect("查表");
            assert_eq!(n, 1, "迁移后必须有表 {t}");
        }

        let v1: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=1")
            .fetch_one(&pool)
            .await
            .expect("台账应有 1 行");
        assert_eq!(v1, "0001_init");
        let v2: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=2")
            .fetch_one(&pool)
            .await
            .expect("台账应有 2 行");
        assert_eq!(v2, "0002_admin_config");
        let v3: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=3")
            .fetch_one(&pool)
            .await
            .expect("台账应有 3 行");
        assert_eq!(v3, "0003_llm_providers");
        let v4: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=4")
            .fetch_one(&pool)
            .await
            .expect("台账应有 4 行");
        assert_eq!(v4, "0004_expert_persona");
        let v5: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=5")
            .fetch_one(&pool)
            .await
            .expect("台账应有 5 行");
        assert_eq!(v5, "0005_expert_source_template");
        let v6: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=6")
            .fetch_one(&pool)
            .await
            .expect("台账应有 6 行");
        assert_eq!(v6, "0006_teams");
        let v7: String = sqlx::query_scalar("SELECT name FROM schema_version WHERE version=7")
            .fetch_one(&pool)
            .await
            .expect("台账应有 7 行");
        assert_eq!(v7, "0007_mcp_transport_alignment");
    }

    /// 0007 把 mcp_servers 的 transport 对齐前端：'http' 改名为
    /// 'streamable_http'，并补上 cwd / max_concurrent_calls /
    /// enabled_capabilities 三列。
    ///
    /// **搬迁不能丢数据**：老库里 transport='http' 的行必须变成
    /// 'streamable_http'，而不是被 CHECK 拒掉或悄悄消失。
    #[tokio::test]
    async fn mcp_transport_is_renamed_and_the_three_missing_columns_exist() {
        let pool = in_memory().await.expect("内存库");
        // 先只跑到 0006，造一条老格式数据。
        for m in MIGRATIONS.iter().filter(|m| m.version <= 6) {
            run_migration(&pool, m.sql).await.expect("迁移");
        }
        sqlx::query(
            "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
             password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) \
             VALUES (x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0','u','u','U',\
             zeroblob(32),zeroblob(16),'token-only','owner',0,0,0)",
        )
        .execute(&pool)
        .await
        .expect("铺用户");
        sqlx::query(
            "INSERT INTO mcp_servers(user_id,name,transport,url,headers_json,asset_hash,\
             created_at,updated_at) \
             VALUES (x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0','old','http','https://x/mcp','{}',\
             zeroblob(32),0,0)",
        )
        .execute(&pool)
        .await
        .expect("铺老格式 MCP 行");

        // 应用 0007。
        let m7 = MIGRATIONS.last().expect("应有 0007");
        assert_eq!(m7.version, 7, "最后一条应当是 0007");
        run_migration(&pool, m7.sql).await.expect("0007 迁移");

        let t: String =
            sqlx::query_scalar("SELECT transport FROM mcp_servers WHERE name='old'")
                .fetch_one(&pool)
                .await
                .expect("老行必须还在");
        assert_eq!(
            t, "streamable_http",
            "老库的 'http' 必须被改名而不是丢掉：数据丢失是静默的，几个月后才发现"
        );

        // 三列存在且可写。
        sqlx::query(
            "UPDATE mcp_servers SET cwd='/tmp', max_concurrent_calls=4, \
             enabled_capabilities_json='[]' WHERE name='old'",
        )
        .execute(&pool)
        .await
        .expect("三列应可写");

        // 旧枚举必须被拒：'http' 已经不合法了。
        let bad = sqlx::query(
            "UPDATE mcp_servers SET transport='http' WHERE name='old'",
        )
        .execute(&pool)
        .await;
        assert!(bad.is_err(), "'http' 已不是合法 transport，CHECK 必须挡住");

        // 并发上限的边界。
        let bad2 = sqlx::query(
            "UPDATE mcp_servers SET max_concurrent_calls=0 WHERE name='old'",
        )
        .execute(&pool)
        .await;
        assert!(bad2.is_err(), "并发上限为 0 没有意义，CHECK 必须挡住");

        // 三态能力：必须是合法 JSON 数组。
        let bad3 = sqlx::query(
            "UPDATE mcp_servers SET enabled_capabilities_json='不是JSON' WHERE name='old'",
        )
        .execute(&pool)
        .await;
        assert!(bad3.is_err(), "能力列表必须是合法 JSON 数组");
    }
    /// team_slug 默认空串并回填成合法 slug，且未软删行内按用户唯一、
    /// 软删后可以复用同一个 slug。
    #[tokio::test]
    async fn team_slug_is_nullable_description_and_reusable_after_soft_delete() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        let now = now_ms();
        // teams 有两条外键（users、leader_session_id → sessions），先把父行铺上，
        // 否则这条测试测的是外键报错而不是 0006 的列语义。
        sqlx::query(
            "INSERT INTO users (id, username, username_norm, display_name, password_hash, \
             password_salt, password_algo, role, pwd_changed_at, created_at, updated_at) \
             VALUES (zeroblob(16), 'probe', 'probe', '探针', zeroblob(32), \
             zeroblob(16), 'pbkdf2-hmac-sha256$i=600000', 'owner', 0, 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("铺测试用户");

        async fn insert_session(
            pool: &sqlx::SqlitePool,
            seed: u8,
            now: i64,
        ) -> Result<(), sqlx::Error> {
            let sql = format!(
                "INSERT INTO sessions (user_id, id, kind, room_id, workspace_path, \
                 created_at, updated_at, last_active_at) \
                 VALUES (zeroblob(16), \
                 x'{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}', \
                 'solo', 'room-{seed}', 'ws/probe-{seed}', ?, ?, ?)"
            );
            sqlx::query(&sql)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await
                .map(|_| ())
        }

        async fn insert_team(
            pool: &sqlx::SqlitePool,
            seed: u8,
            slug: &str,
            desc: Option<&str>,
            deleted: bool,
            now: i64,
        ) -> Result<(), sqlx::Error> {
            insert_session(pool, seed, now).await?;
            let sql = format!(
                "INSERT INTO teams (user_id, id, name, room_id, leader_session_id, \
                 leader_expert_id, team_slug, description, state, state_changed_at, \
                 created_at, updated_at, deleted_at) \
                 VALUES (zeroblob(16), x'{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}', \
                 '探针团', 'room-{seed}', x'{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}{seed:02x}', \
                 'probe-expert', '{slug}', ?, 'IDLE', ?, ?, ?, ?)"
            );
            sqlx::query(&sql)
                .bind(desc)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(if deleted { Some(now) } else { None })
                .execute(pool)
                .await
                .map(|_| ())
        }

        insert_team(&pool, 0x11, "growth-squad", Some("增长小队"), false, now)
            .await
            .expect("合法行必须能写");
        let got: (Option<String>, String) = sqlx::query_as(
            "SELECT description, team_slug FROM teams WHERE team_slug = 'growth-squad'",
        )
        .fetch_one(&pool)
        .await
        .expect("必须能读回来");
        assert_eq!(got.0.as_deref(), Some("增长小队"));
        assert_eq!(got.1, "growth-squad");

        assert!(
            insert_team(&pool, 0x12, "growth-squad", None, false, now)
                .await
                .is_err(),
            "同一个用户下未软删的 slug 必须唯一（否则会静默建出两个同名团队）"
        );

        // 软删原行后再用同一个 slug 建团：部分唯一索引只约束未软删行，
        // 口径与 experts「软删后同名可重建」一致。
        sqlx::query("UPDATE teams SET deleted_at = ? WHERE name = '探针团'")
            .bind(now)
            .execute(&pool)
            .await
            .expect("软删探针团");
        insert_team(&pool, 0x14, "growth-squad", None, false, now)
            .await
            .expect("🔴 软删后必须能用同一个 team_id 重建");
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM teams WHERE team_slug = 'growth-squad' AND deleted_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("查存活行数");
        assert_eq!(live, 1, "重建后必须只有一行是活的（另一行仍软删可追溯）");

        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM teams WHERE description IS NULL")
            .fetch_one(&pool)
            .await
            .expect("查 NULL 语义");
        assert_eq!(n, 1, "description 必须允许 NULL（契约里它可省略）");

        // 老行（0006 之前写入的，ALTER 之后 team_slug 是空串）必须被回填成合法
        // slug，否则这些行对 CRUD 接口等于不存在。这里直接执行迁移文件里那条
        // 回填语句，而不是把它的结果抄一遍断言。
        insert_session(&pool, 0x00, now)
            .await
            .expect("铺老行的主持人会话");
        sqlx::query(
            "INSERT INTO teams (user_id, id, name, room_id, leader_session_id, leader_expert_id, \
             team_slug, description, state, state_changed_at, created_at, updated_at) \
             VALUES (zeroblob(16), x'00000000000000000000000000000000', '老团', \
             'room-legacy', x'00000000000000000000000000000000', 'probe-expert', \
             '', NULL, 'IDLE', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("老行写入");

        let backfill = split_statements(MIGRATION_0006)
            .into_iter()
            .find(|s| s.starts_with("UPDATE teams SET team_slug"))
            .expect("0006 必须含一条 team_slug 回填语句（否则老行对 CRUD 不可见）");
        sqlx::query(&backfill)
            .execute(&pool)
            .await
            .expect("回填必须可执行");
        let filled: String = sqlx::query_scalar("SELECT team_slug FROM teams WHERE name = '老团'")
            .fetch_one(&pool)
            .await
            .expect("回填后必须能读出 slug");
        assert!(!filled.is_empty(), "老行的 slug 不能还是空串：{filled}");
        let shape_ok = filled.len() <= 64
            && !filled.starts_with('-')
            && !filled.ends_with('-')
            && filled
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        assert!(
            shape_ok,
            "回填出来的 slug 必须符合 kebab-case 规则，否则写路径会拒绝它：{filled}"
        );
    }

    /// 成员关系表必须让「同一专家在同一团队里只出现一次」由 schema 兜住，
    /// 写路径漏判重复时不能静默多出一行。
    #[tokio::test]
    async fn team_members_rejects_the_same_expert_twice_in_one_team() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        let now = now_ms();
        sqlx::query(
            "INSERT INTO users (id, username, username_norm, display_name, password_hash, \
             password_salt, password_algo, role, pwd_changed_at, created_at, updated_at) \
             VALUES (zeroblob(16), 'probe', 'probe', '探针', zeroblob(32), \
             zeroblob(16), 'pbkdf2-hmac-sha256$i=600000', 'owner', 0, 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("铺测试用户");
        sqlx::query(
            "INSERT INTO sessions (user_id, id, kind, room_id, workspace_path, created_at, \
             updated_at, last_active_at) \
             VALUES (zeroblob(16), x'11111111111111111111111111111111', \
             'solo', 'room-1', 'ws/probe', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("铺主持人会话");
        sqlx::query(
            "INSERT INTO teams (user_id, id, name, room_id, leader_session_id, leader_expert_id, \
             team_slug, state, state_changed_at, created_at, updated_at) \
             VALUES (zeroblob(16), x'22222222222222222222222222222222', '探针团', \
             'room-1', x'11111111111111111111111111111111', 'cost-analyst', 'growth-squad', \
             'IDLE', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("铺测试团队");

        let insert = "INSERT INTO team_members (user_id, team_id, expert_id, role, state, \
             state_changed_at, joined_at, created_at, updated_at) \
             VALUES (zeroblob(16), x'22222222222222222222222222222222', \
             'growth-analyst', 'member', 'IDLE', ?, ?, ?, ?)";
        sqlx::query(insert)
            .bind(now)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .expect("首个成员必须能写");
        assert!(
            sqlx::query(insert)
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&pool)
                .await
                .is_err(),
            "🔴 同一专家在同一团队里写第二次必须被主键拒掉，否则成员数会虚高"
        );
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM team_members")
            .fetch_one(&pool)
            .await
            .expect("查成员数");
        assert_eq!(n, 1);

        // 同一个专家可以进别的团队（1:N，不该有全局唯一约束）。
        sqlx::query(
            "INSERT INTO teams (user_id, id, name, room_id, leader_session_id, leader_expert_id, \
             team_slug, state, state_changed_at, created_at, updated_at) \
             VALUES (zeroblob(16), x'33333333333333333333333333333333', '另一个团', \
             'room-2', x'11111111111111111111111111111111', 'risk-reviewer', 'risk-squad', \
             'IDLE', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("第二个团队必须能写");
        sqlx::query(
            "INSERT INTO team_members (user_id, team_id, expert_id, role, state, \
             state_changed_at, joined_at, created_at, updated_at) \
             VALUES (zeroblob(16), x'33333333333333333333333333333333', \
             'growth-analyst', 'member', 'IDLE', ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("一个专家可以加入多个团队");
    }

    /// 0004 给 experts 补的 instructions / model 两列：老行必须有默认值，
    /// model 必须允许 NULL（NULL = 跟随实例默认模型），非法值必须被 CHECK 拒掉。
    #[tokio::test]
    async fn expert_persona_columns_have_the_null_semantics_we_promised() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        async fn insert(pool: &sqlx::SqlitePool, id: &str, instructions: &str, model: Option<&str>) -> Result<(), sqlx::Error> {
            let sql = format!(
                "INSERT INTO experts (id, owner_user_id, display_name, version, description, \
                 role_summary, visibility, tool_policy_json, tags_json, license, \
                 default_enabled, is_builtin, asset_hash, persona_hash, skill_count, \
                 created_at, updated_at, deleted_at, instructions, model) \
                 VALUES ('{id}', x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0', '探针', '0.1.0', '', '', \
                 'user_authored', '{{}}', '[]', '', 1, 0, zeroblob(32), zeroblob(32), 0, 0, 0, \
                 NULL, ?, ?)"
            );
            sqlx::query(&sql)
                .bind(instructions)
                .bind(model)
                .execute(pool)
                .await
                .map(|_| ())
        }

        insert(&pool, "probe", "你是成本分析师", Some("qwen3-max"))
            .await
            .expect("合法行必须能写");
        let got: (String, Option<String>) =
            sqlx::query_as("SELECT instructions, model FROM experts WHERE id = 'probe'")
                .fetch_one(&pool)
                .await
                .expect("必须能读回来");
        assert_eq!(got.0, "你是成本分析师");
        assert_eq!(got.1.as_deref(), Some("qwen3-max"));

        insert(&pool, "probe-blank", "", None)
            .await
            .expect("空人格 + NULL 模型必须合法");
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM experts WHERE instructions = '' AND model IS NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("查 NULL 语义");
        assert_eq!(n, 1, "model 必须允许 NULL（= 跟随实例默认模型）");

        assert!(
            insert(&pool, "probe-blank-model", "人格", Some("   "))
                .await
                .is_err(),
            "trim 后为空的模型名必须被 CHECK 拒掉（否则等于把模型切成空串）"
        );
        assert!(
            insert(&pool, "probe-huge", "人".repeat(20_001).as_str(), None)
                .await
                .is_err(),
            "instructions 超过 20000 字符必须被 CHECK 拒掉"
        );
        assert!(
            insert(&pool, "probe-edge", "人".repeat(20_000).as_str(), None)
                .await
                .is_ok(),
            "20000 字符（上限本身）必须放行"
        );

        let ddl: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name='experts'")
            .fetch_one(&pool)
            .await
            .expect("读建表语句");
        assert!(
            ddl.contains("instructions") && ddl.contains("model"),
            "experts 的建表语句里必须看得到这两列（0004 是 ALTER，旧库升级后也在）"
        );
    }

    /// 0005 的 source_template：NULL = 不来自模板；取值必须是 `^[a-z0-9-]{1,64}$`；
    /// 同一个模板 id 必须能落进多行（模板可派生任意多个专家）。
    #[tokio::test]
    async fn expert_source_template_column_allows_null_and_repeats_the_same_template() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        async fn insert(pool: &sqlx::SqlitePool, id: &str, src: Option<&str>) -> Result<(), sqlx::Error> {
            let sql = format!(
                "INSERT INTO experts (id, owner_user_id, display_name, version, description, \
                 role_summary, visibility, tool_policy_json, tags_json, license, \
                 default_enabled, is_builtin, asset_hash, persona_hash, skill_count, \
                 created_at, updated_at, deleted_at, source_template) \
                 VALUES ('{id}', x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0', '探针', '0.1.0', '', '', \
                 'user_authored', '{{}}', '[]', '', 1, 0, zeroblob(32), zeroblob(32), 0, 0, 0, \
                 NULL, ?)"
            );
            sqlx::query(&sql)
                .bind(src)
                .execute(pool)
                .await
                .map(|_| ())
        }

        insert(&pool, "prog-1", Some("ai-coding-coach"))
            .await
            .expect("合法模板 id 必须能写");
        insert(&pool, "prog-2", Some("ai-coding-coach"))
            .await
            .expect("🔴 同一模板必须能派生多个专家（1:N，不该有唯一约束）");
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM experts WHERE source_template = 'ai-coding-coach'",
        )
        .fetch_one(&pool)
        .await
        .expect("查派生数");
        assert_eq!(n, 2, "两个专家都必须记住自己来自 ai-coding-coach");

        let got: Option<String> =
            sqlx::query_scalar("SELECT source_template FROM experts WHERE id = 'prog-1'")
                .fetch_one(&pool)
                .await
                .expect("必须能读回来");
        assert_eq!(got.as_deref(), Some("ai-coding-coach"));

        insert(&pool, "handmade", None)
            .await
            .expect("NULL = 不来自模板，必须合法");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM experts WHERE source_template IS NULL")
            .fetch_one(&pool)
            .await
            .expect("查 NULL 语义");
        assert_eq!(n, 1, "source_template 必须允许 NULL（手建 / 内置专家）");

        insert(&pool, "edge-64", Some(&"a".repeat(64)))
            .await
            .expect("64 个字符（上限本身）必须放行");
        let too_long = "a".repeat(65);
        for (label, bad) in [
            ("空串", ""),
            ("大写", "AI-Coding"),
            ("下划线", "ai_coding"),
            ("含空格", "ai coding"),
            ("中文", "编程教练"),
            ("65 个字符", too_long.as_str()),
        ] {
            assert!(
                insert(&pool, &format!("bad-{}", label), Some(bad)).await.is_err(),
                "不合法的模板 id {label}（{bad:?}）必须被 CHECK 拒掉"
            );
        }

        let ddl: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name='experts'")
            .fetch_one(&pool)
            .await
            .expect("读建表语句");
        assert!(
            ddl.contains("source_template"),
            "experts 的建表语句里必须看得到这一列（0005 是 ALTER，旧库升级后也在）"
        );
    }

    #[tokio::test]
    async fn admin_config_table_rejects_invalid_protocol() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");
        let res = sqlx::query(
            "INSERT INTO admin_config(id,protocol,base_url,api_key,model,\
             max_context_tokens,compaction_threshold_tokens,max_output_tokens,updated_at)\
             VALUES(1,'bogus','http://x/v1','','m',32768,8000,2048,0)",
        )
        .execute(&pool)
        .await;
        assert!(res.is_err(), "protocol 必须在 enum 内，否则 CHECK 拒绝");
    }

    #[tokio::test]
    async fn admin_config_table_rejects_threshold_above_context() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");
        let res = sqlx::query(
            "INSERT INTO admin_config(id,protocol,base_url,api_key,model,\
             max_context_tokens,compaction_threshold_tokens,max_output_tokens,updated_at)\
             VALUES(1,'openai','http://x/v1','','m',8000,16000,2048,0)",
        )
        .execute(&pool)
        .await;
        assert!(
            res.is_err(),
            "compaction_threshold_tokens > max_context_tokens 必须被拒"
        );
    }

    /// 0003 的 llm_providers：合法行必须能写进去，越界 / 非法枚举的行必须被 CHECK 拒掉。
    /// 这些约束是「绕过 API 直写 INSERT」这条路径上唯一的守门人，所以要逐条钉死。
    #[tokio::test]
    async fn llm_providers_table_enforces_its_check_constraints() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        async fn insert(
            pool: &sqlx::SqlitePool,
            kind: &str,
            protocol: &str,
            ctx: i64,
            compaction: i64,
            output: i64,
        ) -> Result<(), sqlx::Error> {
            sqlx::query(
                "INSERT INTO llm_providers(id,name,preset_id,kind,protocol,base_url,api_key,\
                 model,max_context_tokens,compaction_threshold_tokens,max_output_tokens,\
                 enabled,is_default,created_at,updated_at)\
                 VALUES('A1','本地','custom',?1,?2,'http://x/v1','k','m',?3,?4,?5,1,0,0,0)",
            )
            .bind(kind)
            .bind(protocol)
            .bind(ctx)
            .bind(compaction)
            .bind(output)
            .execute(pool)
            .await
            .map(|_| ())
        }

        insert(&pool, "custom", "openai", 32768, 8000, 4096)
            .await
            .expect("合法行必须能写进去");

        let cases: [(&str, &str, i64, i64, i64); 6] = [
            ("custom", "openrouter", 32768, 8000, 4096),
            ("magic", "openai", 32768, 8000, 4096),
            ("custom", "openai", 0, 8000, 4096),
            ("custom", "openai", 8000, 16000, 4096),
            ("custom", "openai", 32768, 4000, 4096),
            ("custom", "openai", 32768, 8000, 65536),
        ];
        for (kind, protocol, ctx, compaction, output) in cases {
            let res = insert(&pool, kind, protocol, ctx, compaction, output).await;
            assert!(
                res.is_err(),
                "kind={kind} protocol={protocol} ctx={ctx} compaction={compaction} \
                 output={output} 必须被 CHECK 拒绝"
            );
        }
    }

    #[tokio::test]
    async fn edited_migration_is_refused_instead_of_silently_reapplied() {
        let pool = in_memory().await.expect("内存库");
        migrate(&pool).await.expect("迁移");

        sqlx::query("UPDATE schema_version SET checksum = x'00' WHERE version = 1")
            .execute(&pool)
            .await
            .expect("篡改校验和");

        let err = migrate(&pool).await.expect_err("校验和不符必须拒绝");
        assert!(
            matches!(err, MigrateError::Drift { version: 1, .. }),
            "必须是 Drift，实际 {err:?}"
        );
        assert!(
            err.to_string().contains("补一条新的迁移"),
            "错误必须给出修复方向：{err}"
        );
    }

    #[test]
    fn checksum_is_stable_and_label_sensitive() {
        assert_eq!(checksum("CREATE TABLE a(x);"), checksum("CREATE TABLE a(x);"));
        assert_ne!(checksum("CREATE TABLE a(x);"), checksum("CREATE TABLE a(y);"));
    }
}
