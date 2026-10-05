
use quill_backup::BackupSource;
use quill_store::configure_pool;
use quill_upgrade::{take_pre_upgrade_backup, UpgradeError};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "quill-upgrade-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("建测试工作区");
    base
}

async fn seeded(tag: &str) -> (SqlitePool, PathBuf, PathBuf) {
    let ws = workspace(tag);
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    std::fs::create_dir_all(data_root.join("u1/wiki")).expect("建数据目录");

    let pool = configure_pool(&db_path.to_string_lossy(), 1)
        .await
        .expect("建连接池");
    sqlx::query(
        "CREATE TABLE notes (\
           id INTEGER PRIMARY KEY, \
           body TEXT NOT NULL, \
           created_unix INTEGER NOT NULL\
         )",
    )
    .execute(&pool)
    .await
    .expect("建表");

    for i in 1..=25i64 {
        sqlx::query("INSERT INTO notes (id, body, created_unix) VALUES (?, ?, ?)")
            .bind(i)
            .bind(format!("第 {i} 条 · 中文 & < > \" ' 都要经得起字节级比对"))
            .bind(1_757_030_000i64 + i)
            .execute(&pool)
            .await
            .expect("写入笔记");
    }
    std::fs::write(
        data_root.join("u1/wiki/index.md"),
        "# 索引\n\n- [[页面 A]]\n",
    )
    .expect("写 wiki");
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
        .expect("checkpoint");
    (pool, db_path, data_root)
}

async fn all_bodies(db_path: &Path) -> Vec<String> {
    let pool = SqlitePool::connect(&format!("sqlite://{}", db_path.display()))
        .await
        .expect("重开库");
    sqlx::query("SELECT body FROM notes ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("读笔记")
        .into_iter()
        .map(|r| r.get::<String, _>(0))
        .collect()
}

fn source(pool: SqlitePool, db_path: &Path, data_root: &Path) -> BackupSource {
    BackupSource::new(pool, db_path, data_root).expect("构造备份源")
}

fn backup_dir(data_root: &Path) -> PathBuf {
    data_root
        .parent()
        .expect("data_root 必有父目录")
        .join("bak")
}

#[tokio::test]
async fn rollback_restores_db_exactly_and_flags_modified_files() {
    let (pool, db_path, data_root) = seeded("rollback").await;
    let src = source(pool.clone(), &db_path, &data_root);

    let before = all_bodies(&db_path).await;
    assert_eq!(before.len(), 25, "装置自检：升级前应有 25 条笔记");

    let mut guard = take_pre_upgrade_backup(&src, backup_dir(&data_root), "1.0.0")
        .await
        .expect("升级前备份必须成功");

    sqlx::query("DELETE FROM notes WHERE id > 5")
        .execute(&pool)
        .await
        .expect("破坏数据");
    sqlx::query("INSERT INTO notes (id, body, created_unix) VALUES (999, '半途而废的写入', 1)")
        .execute(&pool)
        .await
        .expect("写入残留");
    let wiki = data_root.join("u1/wiki/index.md");
    std::fs::write(&wiki, "# 被升级写坏\n").expect("原地改坏 wiki");

    pool.close().await;

    let report = guard.rollback().await.expect("回滚必须成功");
    assert_eq!(report.db_path, db_path, "回滚必须写回原库路径");

    let after = all_bodies(&db_path).await;
    assert_eq!(
        before, after,
        "回滚后的笔记内容与升级前不一致 —— 这正是本 crate 存在的理由"
    );
    assert!(
        !after.iter().any(|b| b.contains("半途而废")),
        "迁移中断的残留行竟然活过了回滚"
    );

    assert_eq!(
        guard.unrestored_files(),
        &["u1/wiki/index.md".to_string()],
        "原地被改坏的文件既没被回滚、也没被上报 —— 这正是静默失败"
    );
    assert_eq!(
        std::fs::read_to_string(&wiki).expect("读回 wiki"),
        "# 被升级写坏\n",
        "回滚不该覆盖升级后已存在的文件（它可能是用户新建的数据）"
    );
}

#[tokio::test]
async fn rollback_recreates_files_deleted_by_upgrade() {
    let (pool, db_path, data_root) = seeded("deleted").await;
    let src = source(pool.clone(), &db_path, &data_root);
    let mut guard = take_pre_upgrade_backup(&src, backup_dir(&data_root), "1.0.0")
        .await
        .expect("升级前备份必须成功");

    let wiki = data_root.join("u1/wiki/index.md");
    assert!(wiki.is_file(), "装置自检：备份前该文件应存在");
    std::fs::remove_file(&wiki).expect("模拟升级误删文件");
    pool.close().await;

    let report = guard.rollback().await.expect("回滚必须成功");
    assert!(
        report.not_overwritten.is_empty(),
        "文件是被删而非被改，不该进 not_overwritten：{:?}",
        report.not_overwritten
    );
    assert!(guard.unrestored_files().is_empty(), "没有未恢复文件");
    assert_eq!(
        std::fs::read_to_string(&wiki).expect("文件应被重建"),
        "# 索引\n\n- [[页面 A]]\n",
        "被升级删掉的文件没有被回滚重建"
    );
}

#[tokio::test]
async fn rollback_is_idempotent() {
    let (pool, db_path, data_root) = seeded("idem").await;
    let src = source(pool.clone(), &db_path, &data_root);
    let mut guard = take_pre_upgrade_backup(&src, backup_dir(&data_root), "1.0.0")
        .await
        .expect("升级前备份必须成功");

    sqlx::query("DELETE FROM notes")
        .execute(&pool)
        .await
        .expect("清空");
    pool.close().await;

    let first = guard.rollback().await.expect("第一次回滚");
    let second = guard.rollback().await.expect("第二次回滚（必须也能成功）");

    assert_eq!(first.db_path, second.db_path);
    assert_eq!(
        first.restored, second.restored,
        "两次回滚恢复的文件集合应一致"
    );
    assert!(
        !first.restored.is_empty(),
        "装置自检：回滚应真的恢复了文件，空集合说明可能在测空气"
    );
    assert_eq!(all_bodies(&db_path).await.len(), 25, "回滚后数据应完好");
}

#[tokio::test]
async fn failed_backup_yields_no_guard() {
    let (pool, db_path, data_root) = seeded("failfast").await;
    let src = source(pool.clone(), &db_path, &data_root);

    let dest = backup_dir(&data_root);
    std::fs::create_dir_all(&dest).expect("建非空目标");
    std::fs::write(dest.join("占位.txt"), "已有内容").expect("占位");

    let err = take_pre_upgrade_backup(&src, &dest, "1.0.0")
        .await
        .expect_err("向非空目录备份必须失败");
    assert!(
        matches!(err, UpgradeError::PreUpgradeBackupFailed { .. }),
        "错误类型不对：{err:?}"
    );

    let msg = err.to_string();
    assert!(
        msg.contains(&dest.display().to_string()),
        "错误信息里没有现场路径，用户无从下手：{msg}"
    );
    assert!(
        msg.contains("quill doctor"),
        "错误信息未指向诊断命令：{msg}"
    );
    pool.close().await;
}

#[tokio::test]
async fn missing_db_fails_before_any_backup() {
    let ws = workspace("missing");
    let data_root = ws.join("data");
    std::fs::create_dir_all(&data_root).expect("建数据目录");
    let ghost_db = ws.join("不存在的.db");
    assert!(!ghost_db.is_file(), "装置自检：该库本应不存在");

    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("建内存池（只为满足构造签名）");
    assert!(
        BackupSource::new(pool, &ghost_db, &data_root).is_err(),
        "源库不存在时构造备份源竟成功了"
    );
}
