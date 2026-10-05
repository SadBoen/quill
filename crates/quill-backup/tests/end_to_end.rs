
use quill_backup::{
    create_backup, restore_backup, BackupError, BackupSource, ExcludedEntry, Manifest,
    MANIFEST_NAME,
};
use quill_store::configure_pool;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "quill-backup-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("建测试工作区");
    base
}

async fn seeded_pool(db_path: &Path) -> SqlitePool {
    let pool = configure_pool(&db_path.to_string_lossy(), 1)
        .await
        .expect("建连接池");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS notes (\
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
            .bind(format!(
                "第 {i} 条笔记 · 中文内容与符号 & < > \" ' 都要经得起字节级比对"
            ))
            .bind(1_757_030_000i64 + i)
            .execute(&pool)
            .await
            .expect("写入笔记");
    }

    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
        .expect("checkpoint");
    pool
}

fn seed_data_root(root: &Path) {
    let u1 = root.join("user-a");
    std::fs::create_dir_all(u1.join("wiki/wiki")).expect("建 wiki 目录");
    std::fs::create_dir_all(u1.join("sessions/s1")).expect("建会话目录");
    std::fs::create_dir_all(u1.join("wiki/raw")).expect("建 raw 目录");

    std::fs::write(
        u1.join("wiki/wiki/index.md"),
        "# 索引\n\n- [条目一](a.md)\n- [条目二](b.md)\n",
    )
    .expect("写 wiki 索引");
    std::fs::write(u1.join("wiki/wiki/a.md"), "# 条目一\n\n正文，含中文。\n").expect("写 a.md");
    std::fs::write(u1.join("wiki/wiki/b.md"), "# 条目二\n\n另一段正文。\n").expect("写 b.md");
    std::fs::write(u1.join("wiki/raw/paper.pdf"), b"%PDF-1.4\nfake\n").expect("写 raw 文件");

    let bin: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
    std::fs::write(u1.join("sessions/s1/history.bin"), &bin).expect("写二进制会话");

    let enc: Vec<u8> = (0..1024u32)
        .map(|i| (i.wrapping_mul(7) % 256) as u8)
        .collect();
    std::fs::write(u1.join("secrets.enc"), &enc).expect("写凭据密文");

    std::fs::write(u1.join("master.key"), b"THIS-MUST-NEVER-BE-BACKED-UP").expect("写密钥文件");

    std::fs::create_dir_all(root.join("user-b/empty-dir")).expect("建空目录");

    std::fs::write(root.join("user-b/config.toml"), "name = \"b\"\n").expect("写配置");
}

fn snapshot_tree(root: &Path) -> std::collections::BTreeMap<String, Option<Vec<u8>>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读目录 {} 失败：{e}", dir.display()));
        for e in rd {
            let p = e.expect("读目录项").path();
            let rel = p
                .strip_prefix(root)
                .expect("路径在 root 内")
                .to_string_lossy()
                .replace('\\', "/");
            let meta = std::fs::symlink_metadata(&p).expect("读属性");
            if meta.is_dir() {
                out.insert(rel.clone(), None);
                stack.push(p);
            } else {
                out.insert(rel, Some(std::fs::read(&p).expect("读文件")));
            }
        }
    }
    out
}

fn files_only(
    tree: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
) -> std::collections::BTreeMap<String, Vec<u8>> {
    tree.iter()
        .filter_map(|(k, v)| v.as_ref().map(|b| (k.clone(), b.clone())))
        .collect()
}

fn dirs_only(
    tree: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
) -> std::collections::BTreeSet<String> {
    tree.iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.clone())
        .collect()
}

fn expected_tree(
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> std::collections::BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .filter(|(k, _)| !k.ends_with("master.key"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

async fn read_notes(pool: &SqlitePool) -> Vec<(i64, String, i64)> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, body, created_unix FROM notes ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("读笔记");
    rows.into_iter()
        .map(|r| {
            (
                r.get::<i64, _>(0),
                r.get::<String, _>(1),
                r.get::<i64, _>(2),
            )
        })
        .collect()
}

#[tokio::test]
async fn 造数据备份破坏恢复后数据与备份前完全一致() {
    let ws = workspace("e2e-full");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);

    let pool = seeded_pool(&db_path).await;
    let notes_before = read_notes(&pool).await;
    let tree_before = snapshot_tree(&data_root);
    let files_before = files_only(&tree_before);
    assert_eq!(notes_before.len(), 25, "造的数据不对，测试本身失去意义");
    assert!(
        files_before.contains_key("user-a/master.key"),
        "造数据阶段必须真的放了密钥文件，否则「密钥被排除」这条断言是恒绿"
    );

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    let report = create_backup(&src, &backup_dir).await.expect("备份应成功");

    assert_eq!(
        report.manifest.files.len(),
        files_before.len() - 1,
        "应除 master.key 外全部收录（树里 {:#?}）",
        files_before.keys().collect::<Vec<_>>()
    );
    assert!(
        std::fs::read(backup_dir.join("MANIFEST")).is_ok(),
        "备份目录必须有清单文件"
    );

    sqlx::query("DELETE FROM notes")
        .execute(&pool)
        .await
        .expect("清空笔记");
    sqlx::query("INSERT INTO notes (id, body, created_unix) VALUES (999, '垃圾数据', 0)")
        .execute(&pool)
        .await
        .expect("塞垃圾行");
    std::fs::remove_dir_all(&data_root).expect("删掉整个数据目录");
    std::fs::create_dir_all(&data_root).expect("重建空数据目录");

    assert_eq!(read_notes(&pool).await.len(), 1, "破坏未生效，测试失去意义");
    assert!(snapshot_tree(&data_root).is_empty(), "破坏未生效");

    drop(pool);
    let restored_db = ws.join("restored/quill.db");
    let restored_root = ws.join("restored/data");
    let rr = restore_backup(&backup_dir, &restored_db, &restored_root)
        .await
        .expect("恢复应成功");

    assert!(
        rr.not_overwritten.is_empty(),
        "目标本应是空的，不该有未覆盖项"
    );
    assert_eq!(rr.restored.len(), report.manifest.files.len());

    let pool2 = configure_pool(&restored_db.to_string_lossy(), 1)
        .await
        .expect("打开恢复后的库");
    let notes_after = read_notes(&pool2).await;
    assert_eq!(
        notes_after, notes_before,
        "恢复后的数据库内容与备份前不一致（垃圾行未被清除，或有行丢失）"
    );

    let tree_after = snapshot_tree(&restored_root);

    let expected = expected_tree(&files_before);
    assert_eq!(
        files_only(&tree_after),
        expected,
        "恢复后的用户文件与备份前不一致（目录键不应出现在文件集里）"
    );

    let mut expected_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for rel in expected.keys() {
        let mut p = rel.as_str();
        while let Some(idx) = p.rfind('/') {
            expected_dirs.insert(p[..idx].to_string());
            p = &p[..idx];
        }
        expected_dirs.insert(p.to_string());
    }
    assert_eq!(
        dirs_only(&tree_after),
        expected_dirs,
        "恢复后的目录集合与「文件路径推导出的目录集合」不一致"
    );

    let enc_before = tree_before
        .get("user-a/secrets.enc")
        .and_then(|v| v.as_ref())
        .expect("备份前应有 secrets.enc");
    let enc_after = tree_after
        .get("user-a/secrets.enc")
        .and_then(|v| v.as_ref())
        .expect("恢复后应有 secrets.enc");
    assert_eq!(
        enc_after, enc_before,
        "secrets.enc 密文被改动（应原样搬运）"
    );

    pool2.close().await;
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 同一份备份恢复两次结果完全相同() {
    let ws = workspace("e2e-idempotent");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;
    let notes_before = read_notes(&pool).await;

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    create_backup(&src, &backup_dir).await.expect("备份应成功");
    drop(pool);

    let r1_root = ws.join("r1/data");
    let r2_root = ws.join("r2/data");
    restore_backup(&backup_dir, &ws.join("r1/quill.db"), &r1_root)
        .await
        .expect("第一次恢复应成功");
    restore_backup(&backup_dir, &ws.join("r2/quill.db"), &r2_root)
        .await
        .expect("第二次恢复应成功");

    assert_eq!(
        snapshot_tree(&r1_root),
        snapshot_tree(&r2_root),
        "恢复两次得到的数据树不同 —— 恢复不幂等"
    );

    let p1 = configure_pool(&ws.join("r1/quill.db").to_string_lossy(), 1)
        .await
        .expect("打开 r1 库");
    let p2 = configure_pool(&ws.join("r2/quill.db").to_string_lossy(), 1)
        .await
        .expect("打开 r2 库");
    assert_eq!(
        read_notes(&p1).await,
        read_notes(&p2).await,
        "恢复两次得到的库内容不同 —— 恢复不幂等"
    );
    assert_eq!(read_notes(&p1).await, notes_before);
    p1.close().await;
    p2.close().await;
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 空目录不进清单恢复后不重建但有文件内容的目录都在() {

    let ws = workspace("e2e-emptydir");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    create_backup(&src, &backup_dir).await.expect("备份应成功");
    drop(pool);

    let t = ws.join("t/data");
    restore_backup(&backup_dir, &ws.join("t/quill.db"), &t)
        .await
        .expect("恢复应成功");

    let dirs = dirs_only(&snapshot_tree(&t));
    assert!(
        !dirs.contains("user-b/empty-dir"),
        "空目录被重建了 —— 若这是有意的改进，请同步更新本用例与 crate 文档"
    );

    for must in [
        "user-a",
        "user-a/wiki",
        "user-a/wiki/wiki",
        "user-a/wiki/raw",
        "user-a/sessions",
        "user-a/sessions/s1",
        "user-b",
    ] {
        assert!(dirs.contains(must), "目录 {must} 未被重建：{dirs:?}");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 恢复到已有数据的目录时同内容覆盖而不同内容保留() {

    let ws = workspace("e2e-keep");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;
    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    create_backup(&src, &backup_dir).await.expect("备份应成功");
    drop(pool);

    let t = ws.join("target/data");
    std::fs::create_dir_all(t.join("user-a/wiki/wiki")).expect("建目标目录");
    std::fs::write(t.join("user-a/wiki/wiki/a.md"), "我自己后来写的内容\n").expect("写 a.md");
    std::fs::copy(
        data_root.join("user-a/wiki/wiki/index.md"),
        t.join("user-a/wiki/wiki/index.md"),
    )
    .expect("复制相同内容的 index.md");

    let rr = restore_backup(&backup_dir, &ws.join("target/quill.db"), &t)
        .await
        .expect("恢复应成功");
    assert_eq!(
        rr.not_overwritten,
        vec!["user-a/wiki/wiki/a.md".to_string()],
        "内容不同的文件必须被保留而不是静默覆盖"
    );
    assert_eq!(
        std::fs::read_to_string(t.join("user-a/wiki/wiki/a.md")).expect("读回"),
        "我自己后来写的内容\n",
        "内容不同的文件被覆盖了 —— 静默丢数据"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 备份文件被篡改时恢复拒绝且目标目录一个字节都没被改() {
    let ws = workspace("e2e-tamper");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;
    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    create_backup(&src, &backup_dir).await.expect("备份应成功");
    drop(pool);

    let victim = backup_dir.join("data/user-a/wiki/wiki/a.md");
    let mut bytes = std::fs::read(&victim).expect("读被篡改文件");
    bytes[0] = b'X';
    std::fs::write(&victim, &bytes).expect("写回被篡改文件");

    let t = ws.join("target/data");
    std::fs::create_dir_all(&t).expect("建目标目录");
    let sentinel = t.join("sentinel.txt");
    std::fs::write(&sentinel, "别动我").expect("写哨兵");

    let err = restore_backup(&backup_dir, &ws.join("target/quill.db"), &t)
        .await
        .expect_err("被篡改的备份必须恢复失败");
    match err {
        BackupError::DigestMismatch { ref rel, .. } => {
            assert_eq!(rel, "user-a/wiki/wiki/a.md");
        }
        other => panic!("应报摘要不符，实际 {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("读哨兵"),
        "别动我",
        "校验失败时目标目录被改了 —— 留下半开状态"
    );
    assert!(
        !ws.join("target/quill.db").exists(),
        "校验失败时数据库已被写入 —— 留下半开状态"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 缺少清单时恢复拒绝并给出可复制命令() {
    let ws = workspace("e2e-nomanifest");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&backup_dir).expect("建备份目录");
    let t = ws.join("target/data");
    std::fs::create_dir_all(&t).expect("建目标目录");

    let err = restore_backup(&backup_dir, &ws.join("t/quill.db"), &t)
        .await
        .expect_err("没有清单必须拒绝");
    assert!(matches!(err, BackupError::ManifestMissing { .. }));
    let rendered = err.render_with_fix();
    assert!(
        rendered.contains("请执行"),
        "错误必须给出修复命令：{rendered}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 同一份源连备两次得到相同的清单内容() {
    let ws = workspace("e2e-twice");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;
    let notes = read_notes(&pool).await;

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    let r1 = create_backup(&src, ws.join("b1"))
        .await
        .expect("第一次备份");
    let r2 = create_backup(&src, ws.join("b2"))
        .await
        .expect("第二次备份");

    assert_eq!(
        r1.manifest.db_sha256, r2.manifest.db_sha256,
        "同源两次备份的数据库快照摘要应相同 —— VACUUM INTO 不确定性"
    );
    assert_eq!(
        r1.manifest.files, r2.manifest.files,
        "同源两次备份的文件清单应相同"
    );
    assert_eq!(r1.manifest.total_bytes, r2.manifest.total_bytes);
    assert_eq!(read_notes(&pool).await, notes, "备份不应改动源库");
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 目标目录非空时拒绝覆盖() {
    let ws = workspace("e2e-nonempty");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    std::fs::create_dir_all(&backup_dir).expect("建备份目录");
    std::fs::write(backup_dir.join("上一份备份的痕迹"), "x").expect("造占用文件");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    let err = create_backup(&src, &backup_dir)
        .await
        .expect_err("非空目标必须拒绝");
    assert!(matches!(err, BackupError::DestNotEmpty { .. }));
    assert!(
        err.render_with_fix().contains("mv"),
        "拒绝时应给出可复制的移开命令（不可逆的 rm 不作为首选）"
    );
    assert!(
        backup_dir.join("上一份备份的痕迹").exists(),
        "拒绝覆盖时不得删掉已有内容"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 密钥材料不进备份而凭据密文进备份() {
    let ws = workspace("e2e-secret");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;

    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    let report = create_backup(&src, &backup_dir).await.expect("备份应成功");

    let backup_files = files_only(&snapshot_tree(&backup_dir));
    for (rel, bytes) in &backup_files {
        assert!(
            !bytes.windows(14).any(|w| w == b"THIS-MUST-NEVE"),
            "密钥内容出现在备份的 {rel} 里"
        );
        assert!(
            !rel.contains("master.key"),
            "备份清单里出现了 master.key：{rel}"
        );
    }

    let ex: Vec<&ExcludedEntry> = report.excluded.iter().collect();
    assert!(
        ex.iter()
            .any(|e| e.rel.ends_with("master.key") && e.reason.contains("密钥")),
        "master.key 的排除必须记进清单并说明原因，实际 {:?}",
        report.excluded
    );

    assert!(
        backup_dir.join("data/user-a/secrets.enc").is_file(),
        "凭据密文必须进备份，否则恢复出来的实例没有对端凭据"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn 清单如实记录被排除的条目且可被重新解析() {
    let ws = workspace("e2e-manifest");
    let db_path = ws.join("quill.db");
    let data_root = ws.join("data");
    let backup_dir = ws.join("backup");
    std::fs::create_dir_all(&data_root).expect("建 data 目录");
    seed_data_root(&data_root);
    let pool = seeded_pool(&db_path).await;
    let src = BackupSource::new(pool.clone(), &db_path, &data_root).expect("构造备份源");
    create_backup(&src, &backup_dir).await.expect("备份应成功");

    let text = std::fs::read_to_string(backup_dir.join(MANIFEST_NAME)).expect("读清单");
    let back = Manifest::parse(&text).expect("自己写的清单必须能读回来");
    assert_eq!(back.excluded.len(), 1, "应恰有 1 个排除项（master.key）");
    assert!(back.excluded[0].rel.ends_with("master.key"));

    let names: Vec<&str> = back.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(back.files.len(), 7, "收录文件应恰为 7 个，实际 {:?}", names);
    assert!(!names.iter().any(|n| n.contains("master.key")));
    let _ = std::fs::remove_dir_all(&ws);
}
