//! `/healthz` 的表结构检查（queue Q087）。
//!
//! **为什么要有这个文件**：`ready: true` 是给运维看的一句担保 —— 它说「可以发请求了」。
//! 原来它只查两张表（`experts` / `task_dispatches`），于是 `sessions` 缺失时照样回
//! `ready: true`，而聊天路由一进去就 500：健康检查比它担保的东西弱，等于没担保。
//!
//! 这里钉两条：① 刚迁移完的库不该缺表；② 抽掉一张**已接线**用的表时必须点名它
//! （而不是只查原来那两张，也不是含糊地说一句「schema 不对」）。

mod common;

use common::TestDb;
use quill_server::db::{storage_error, REQUIRED_TABLES};

#[test]
fn a_freshly_migrated_database_reports_no_missing_tables() {
    let t = TestDb::new("healthz-tables-fresh");
    let missing = t.bridge().missing_tables().expect("探测表结构");
    assert!(
        missing.is_empty(),
        "刚跑完迁移的库不该缺表，实测缺：{missing:?}"
    );
}

#[tokio::test]
async fn dropping_a_wired_table_is_named_and_only_it() {
    let t = TestDb::new("healthz-tables-dropped");
    let db = t.bridge();

    // `sessions` 是「接线在用」的表里最典型的一张：会话列表、发消息、改名、
    // 用量页全都要它。抽掉它之后，`/healthz` 必须点名 —— 这正是旧清单漏掉的。
    assert!(
        REQUIRED_TABLES.contains(&"sessions"),
        "这条测试的前提是 `sessions` 在必查清单里"
    );
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query("DROP TABLE sessions")
                .execute(&pool)
                .await
                .map_err(|e| storage_error("抽掉 sessions", e))?;
            Ok(())
        })
    })
    .expect("抽表失败");

    let missing = db.missing_tables().expect("探测表结构");
    assert_eq!(
        missing,
        vec!["sessions".to_string()],
        "缺的表要点名，而且**只**报缺的那些（多报会让运维去修一个没坏的东西）"
    );
}

#[test]
fn every_required_table_is_a_real_table_from_the_migrations() {
    // 必查清单里的名字必须是迁移里真建过的表 —— 写错一个字母的后果是
    // `/healthz` 永远报「缺表」，而那张表其实叫别的名字，运维会找不到。
    let dir = std::path::Path::new("../quill-store/migrations");
    assert!(
        dir.is_dir(),
        "读不到迁移目录（{}）：这条检查的前提是仓库布局没变",
        dir.display()
    );
    let mut schema = String::new();
    for entry in std::fs::read_dir(dir).expect("读迁移目录") {
        let path = entry.expect("目录项").path();
        if path.extension().is_some_and(|e| e == "sql") {
            schema.push_str(&std::fs::read_to_string(&path).expect("读迁移文件"));
        }
    }
    for table in REQUIRED_TABLES {
        assert!(
            schema.contains(&format!("TABLE {table}"))
                || schema.contains(&format!("TABLE IF NOT EXISTS {table}")),
            "必查清单里的 {table:?} 在任何一条迁移里都没有建过"
        );
    }
}
