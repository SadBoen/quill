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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quill-store/migrations");
    // 0001 → 0002 → … 顺序由文件名天然保证；只读这些，不读别的。
    // 少读一个会让新列在测试库里不存在，而测试会假绿。
    let mut buf = String::new();
    for name in [
        "0001_init.sql",
        "0002_admin_config.sql",
        "0003_llm_providers.sql",
        "0004_expert_persona.sql",
        "0005_expert_source_template.sql",
        "0006_teams.sql",
        "0007_mcp_transport_alignment.sql",
    ] {
        let path = dir.join(name);
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
