//! 日志不许打印凭据（queue Q086）。
//!
//! **为什么是源码扫描而不是运行期抓日志**：凭据泄漏是「写代码那一刻」的决定 ——
//! `eprintln!("...{row}")` 这种一行就能把 MCP 的 `env` / `headers`、模型的
//! `api_key`、登录令牌整个打进 stderr，而运行期抓日志要么抓不到那条分支，
//! 要么得把服务真跑起来。这里扫**源码**：凡是日志宏那一行提到凭据类标识符，直接红。
//!
//! 2026-10-08 实测这套扫描在仓库里**一条命中都没有**（也就是说现在的代码是干净的）——
//! 这条测试的价值不在「修了什么」，而在**以后别写出来**：这类东西一旦写进去，
//! 通常是在排障时顺手加的，review 时最不容易被看出来。
//!
//! 两个刻意的设计：
//! - ** needles 在运行期拼**（`format!("api{}", "_key")`）：否则这条测试自己的源码
//!   就会包含那些字面量，扫描器把自己的 doc 判红。
//! - **断言扫到的文件数**：扫目录的检查最常见的坏法是「路径写错 → 一个文件没扫 →
//!   永远绿」。这里要求扫到 50 个以上的 .rs，扫不到就红。

use std::path::{Path, PathBuf};

/// 出现这些宏的行才算「日志行」。`dbg!` 也列上：它同样写 stderr。
const LOG_MACROS: [&str; 6] = [
    "eprintln!",
    "println!",
    "dbg!",
    "log::",
    "tracing::",
    "panic!",
];

fn needles() -> Vec<String> {
    // 运行期拼出来，避免本文件自己命中（见文件头说明）。
    vec![
        format!("api{}_{}", "", "key"),
        format!(".{}", "env"),
        format!(".{}", "headers"),
        format!("{}oken", "t"),
        format!("{}ecret", "s"),
        format!("{}assword", "p"),
        format!("Bea{} ", "rer"),
    ]
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_log_line_mentions_a_credential_identifier() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates 目录")
        .to_path_buf();
    let mut files = Vec::new();
    rs_files(&crates, &mut files);
    assert!(
        files.len() > 50,
        "只扫到 {} 个 .rs —— 扫描路径很可能写错了({})，这种检查最怕「扫了个空还判绿」",
        files.len(),
        crates.display()
    );

    let needles = needles();
    let mut hits = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if !LOG_MACROS.iter().any(|m| line.contains(m)) {
                continue;
            }
            for n in &needles {
                if line.contains(n.as_str()) {
                    hits.push(format!(
                        "{}:{} 命中 {n:?}：{}",
                        f.display(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "日志里出现凭据类标识符（会把 MCP env/headers、api_key、令牌打进 stderr）：\n{}",
        hits.join("\n")
    );
}
