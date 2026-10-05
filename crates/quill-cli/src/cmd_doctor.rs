use crate::{store, Opts, Outcome};
use std::path::Path;

pub async fn run(o: &Opts) -> Outcome {
    let mut lines: Vec<String> = Vec::new();
    let mut worst: u8 = 0;

    let bad = |w: &mut u8, buf: &mut Vec<String>, title: &str, state: char, msg: &str| {
        buf.push(format!("{state} {title}\n      {msg}"));
        match state {
            '✗' => *w = (*w).max(1),
            '▲' => *w = 2,
            _ => {}
        }
    };

    lines.push("quill doctor —— 体检报告".into());
    lines.push(format!("  数据根目录: {}", o.root));
    lines.push(format!("  数据库    : {}", o.db));
    lines.push("".into());

    let dbp = Path::new(&o.db);
    if let Some(parent) = dbp.parent() {
        if parent.as_os_str().is_empty() || parent.is_dir() {
            bad(
                &mut worst,
                &mut lines,
                "数据库所在目录",
                '✓',
                &parent.display().to_string(),
            );
        } else {
            bad(
                &mut worst,
                &mut lines,
                "数据库所在目录",
                '✗',
                &format!(
                    "{} 不存在。下一步：mkdir -p {}",
                    parent.display(),
                    parent.display()
                ),
            );
        }
    }

    match store::open_db(&o.db).await {
        Ok(pool) => {
            bad(
                &mut worst,
                &mut lines,
                "数据库连接",
                '✓',
                &format!("已打开并确认 schema（{}）", dbp.display()),
            );

            let need = ["users", "experts", "task_dispatches", "wiki_index"];
            let mut missing = Vec::new();
            for t in need {
                let n: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                )
                .bind(t)
                .fetch_one(&pool)
                .await
                .unwrap_or(0);
                if n == 0 {
                    missing.push(t);
                }
            }
            if missing.is_empty() {
                bad(
                    &mut worst,
                    &mut lines,
                    "关键表",
                    '✓',
                    &format!("{} 张全在", need.len()),
                );
            } else {
                bad(
                    &mut worst,
                    &mut lines,
                    "关键表",
                    '✗',
                    &format!(
                        "缺 {}。下一步：quill doctor 会自动跑迁移；\
                     若仍缺，检查该文件是不是别的项目的库。",
                        missing.join("、")
                    ),
                );
            }

            match store::data_root(&o.root) {
                Ok(d) => {
                    let writable = {
                        let probe = d.join(".doctor-probe");
                        match std::fs::write(&probe, b"x") {
                            Ok(()) => {
                                let _ = std::fs::remove_file(&probe);
                                true
                            }
                            Err(_) => false,
                        }
                    };
                    if writable {
                        bad(
                            &mut worst,
                            &mut lines,
                            "数据根目录可写",
                            '✓',
                            &d.display().to_string(),
                        );
                    } else {
                        bad(
                            &mut worst,
                            &mut lines,
                            "数据根目录可写",
                            '✗',
                            &format!(
                            "{} 存在但写不进去。\n      下一步：sudo chown -R $(id -u):$(id -g) {}",
                            d.display(),
                            o.root
                        ),
                        );
                    }
                }
                Err(e) => bad(&mut worst, &mut lines, "数据根目录", '▲', &e.message),
            }

            let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
                .fetch_one(&pool)
                .await
                .unwrap_or(-1);
            let experts: i64 = sqlx::query_scalar("SELECT count(*) FROM experts")
                .fetch_one(&pool)
                .await
                .unwrap_or(-1);
            if users < 0 {
                bad(
                    &mut worst,
                    &mut lines,
                    "读取统计",
                    '▲',
                    "查不到（表不可读）",
                );
            } else {
                bad(
                    &mut worst,
                    &mut lines,
                    "数据统计",
                    '✓',
                    &format!("用户 {users} 个 · 专家 {experts} 个"),
                );
            }
        }
        Err(e) => {
            bad(&mut worst, &mut lines, "数据库连接", '▲', &e.message);
        }
    }

    bad(
        &mut worst,
        &mut lines,
        "版本",
        '✓',
        &format!(
            "quill-cli {}（schema 随二进制内嵌）",
            env!("CARGO_PKG_VERSION")
        ),
    );

    lines.push("".into());
    let (mark, tail) = match worst {
        0 => ("✅ 全部正常", "没有发现问题。"),
        1 => ("❌ 有问题", "按上面每条的「下一步」处理。"),
        _ => (
            "▲ 无法判定",
            "有项目**查不出来**——这比查出问题更危险。上面带 ▲ 的项需要人工介入。",
        ),
    };
    lines.push(format!("{mark}：{tail}"));
    lines.push(format!(
        "（统计：✓ 正常 / ✗ 有问题 / ▲ 无法判定；共 {} 项，其中 ▲ {} 项）",
        lines
            .iter()
            .filter(|l| l.starts_with(['✓', '✗', '▲']))
            .count(),
        lines.iter().filter(|l| l.starts_with('▲')).count()
    ));

    let msg = lines.join("\n");
    if worst == 0 {
        Outcome::ok(msg)
    } else {
        Outcome {
            code: worst,
            message: msg,
        }
    }
}
