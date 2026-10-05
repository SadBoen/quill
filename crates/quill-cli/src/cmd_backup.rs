//! `quill backup` / `quill restore` —— 一致性备份与恢复。
//!
//! 走 `quill-backup` 的既有实现（`VACUUM INTO` + 摘要校验 + 幂等），
//! CLI 不重复实现备份逻辑 —— 那是铁律三明令禁止的第二份实现。

use crate::{store, Opts, Outcome};
use quill_backup::{create_backup, restore_backup, BackupSource};

pub async fn backup(o: &Opts) -> Outcome {
    let Some(dest) = o.rest.first().cloned() else {
        return Outcome::fail(String::from("缺参数。用法：quill backup <目标目录>"));
    };
    let root = match store::data_root(&o.root) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let db = std::path::PathBuf::from(&o.db);

    let pool = match store::open_db(&o.db).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    let src = match BackupSource::new(pool, db.clone(), root.clone()) {
        Ok(s) => s,
        Err(e) => return Outcome::undet(format!("备份源不可用：{e}\n这不是「备份成功」。")),
    };

    match create_backup(&src, &dest).await {
        Ok(r) => {
            let mut out = format!(
                "备份完成。\n  目标目录  : {dest}\n  数据库摘要: {}",
                r.db_sha256
            );
            out.push_str(&format!("\n  文件数    : {}", r.manifest.files.len()));
            out.push_str(&format!("\n  总字节    : {}", r.manifest.total_bytes));
            if !r.excluded.is_empty() {
                out.push_str(&format!(
                    "\n  已排除的密钥材料 {} 项（这些**不会**进备份，需另行保管）：",
                    r.excluded.len()
                ));
                for e in r.excluded.iter().take(5) {
                    out.push_str(&format!("\n    · {} —— {}", e.rel, e.reason));
                }
            }
            out.push_str(&format!(
                "\n\n  恢复它：quill restore {dest} --db {} --root {} --yes",
                o.db, o.root
            ));
            Outcome::ok(out)
        }
        Err(e) => Outcome::undet(format!("备份失败：{e}\n可安全重试。")),
    }
}

pub async fn restore(o: &Opts) -> Outcome {
    let src = match o.rest.first() {
        Some(d) => d.clone(),
        None => return Outcome::fail(String::from("缺参数。用法：quill restore <备份目录>")),
    };
    let root = std::path::PathBuf::from(&o.root);
    let db = std::path::PathBuf::from(&o.db);

    if !o.yes {
        return Outcome::ok(format!(
            "即将用 {} 覆盖当前数据（{} / {}）。\n\
             ⚠ 恢复是破坏性的：当前数据会被替换。\n\
             确认请加 --yes：quill restore {} --db {} --root {} --yes",
            src,
            db.display(),
            root.display(),
            src,
            o.db,
            o.root
        ));
    }

    match restore_backup(std::path::Path::new(&src), &db, &root).await {
        Ok(r) => {
            let mut out = format!("恢复完成。\n  数据库 : {}", r.db_path.display());
            out.push_str(&format!("\n  恢复文件: {} 个", r.restored.len()));
            if !r.not_overwritten.is_empty() {
                out.push_str(&format!(
                    "\n  ⚠ 有 {} 个文件**没有**被覆盖（备份之后它们在目标端已存在）：\n",
                    r.not_overwritten.len()
                ));
                for p in r.not_overwritten.iter().take(5) {
                    out.push_str(&format!("    · {p}\n"));
                }
                out.push_str("    状态并未完全回到备份那一刻 —— 请人工确认这些文件。");
            }
            Outcome::ok(out)
        }
        Err(e) => Outcome::undet(format!(
            "恢复失败：{e}\n\
             ⚠ 摘要校验先于任何写入，所以目标目录**一个字节都没被改**。可安全重试。"
        )),
    }
}
