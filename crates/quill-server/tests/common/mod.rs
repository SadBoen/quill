// 共享测试夹具模块：每个集成测试二进制都 `mod common;`，但只用得上其中一部分。
// 不整体放行 dead_code 的话，任一没被某个二进制用到的 pub 助手都会在那里报 warning
// —— 而那是「模块被多个二进制共用」的必然结果，不是死代码。
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use quill_server::db::{storage_error, DbBridge};

static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct TestDb {
    bridge: Option<Arc<DbBridge>>,
    dir: PathBuf,
}

impl TestDb {
    pub fn new(label: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "quill-server-test-{label}-{}-{n}",
            std::process::id()
        ));

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

    pub fn bridge(&self) -> Arc<DbBridge> {
        Arc::clone(self.bridge.as_ref().expect("TestDb 持有桥接"))
    }

    pub fn path(&self) -> String {
        self.dir.join("quill.db").to_string_lossy().to_string()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if let Some(b) = self.bridge.take() {
            drop(b);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn migration_sql() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../quill-store/migrations");
    // 顺序与文件名**一律从 quill_store::MIGRATIONS 取**，不在这里手写。
    //
    // 原来这里是硬编码的 7 个文件名，于是新加 0008 时忘了加进来：
    // 测试库里的 messages 没有新列，`append_message` 的 INSERT 直接报 SQL 错，
    // 一片 HTTP 用例以 503 失败。写死的那份清单**保证**会和真实迁移脱节 ——
    // 所以这里从单一出处推导，少读一个的可能性直接归零。
    let mut buf = String::new();
    for m in quill_store::MIGRATIONS {
        let path = dir.join(format!("{}.sql", m.name));
        buf.push_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("读取迁移文件 {} 失败：{e}", path.display())),
        );
        buf.push('\n');
    }
    buf
}

pub fn scalar_i64(db: &Arc<DbBridge>, sql: &str) -> i64 {
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

/// 铺一个 `users` 行，形状与 `bootstrap::ensure_token_user` 写出来的完全一致
/// （`password_algo='token-only'`，没有口令）。
///
/// 令牌鉴权会回这一行核状态与角色，所以**凡是拿 `QUILL_TOKENS` 令牌发请求的测试
/// 都必须先铺行**。不铺的话令牌会被当成「这个人在库里不存在」而拒，测的就不是
/// 行为而是夹具了 —— 而且这种测试会「因为夹具不对而绿」，最难发现。
pub async fn seed_token_user(
    db: &Arc<DbBridge>,
    id: &quill_domain::UserId,
    username: &str,
    is_admin: bool,
) {
    let id = id.as_bytes().to_vec();
    let username = username.to_string();
    let role = if is_admin { "owner" } else { "member" };
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
                 password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) \
                 VALUES(?,?,?,?,?,?,?,?,0,0,0)",
            )
            .bind(id)
            .bind(&username)
            .bind(&username)
            .bind(&username)
            .bind(b"token-only-no-password".as_slice())
            .bind(b"token-only".as_slice())
            .bind(quill_control::TOKEN_ONLY_ALGO)
            .bind(role)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("铺 token 账号", e))?;
            Ok(())
        })
    })
    .expect("铺 token 账号失败");
}

/// 把账号改成停用，用来验「停用立刻生效」，而不是只在登录那一刻生效。
pub async fn disable_user(db: &Arc<DbBridge>, id: &quill_domain::UserId) {
    let id = id.as_bytes().to_vec();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query("UPDATE users SET status='disabled' WHERE id=?")
                .bind(id)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("停用测试账号", e))?;
            Ok(())
        })
    })
    .expect("停用测试账号失败");
}

#[cfg(test)]
mod tests {
    use super::*;

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
