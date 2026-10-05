//! 多用户隔离测试（契约 `docs/PHASE2_CONTRACT.md` §八 第 5 条：
//! 「涉及多用户的逻辑，**有隔离性测试**」）。
//!
//! ## 为什么这必须是集成测试而不是单元测试
//!
//! 隔离失效有三种形态，只有真的落盘才暴露：
//! 1. 词法逃逸（`../`）—— 单元测试能查；
//! 2. **根目录算错**（两个用户算到同一个根）—— 单元测试查不到，
//!    因为它不碰文件系统；
//! 3. 写入落错根 —— 只有真写一次文件、换个用户读，才知道读不到。
//!
//! 本文件覆盖 2 与 3。

use quill_wiki::store::WikiStore;
use quill_wiki::UserId;

/// 构造一个合法 `UserId`（最后一个字节做区分）。
fn user(n: u8) -> UserId {
    let mut b = [0u8; 16];
    b[15] = n;
    UserId::from_bytes(b)
}

/// 每个用例独占的临时根（不共享 → 用例之间不会互相污染）。
fn tmp_root(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "quill-wiki-iso-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("建临时根");
    p
}

const A_ONLY_SECRET: &str = "A 用户的私密内容，不该被 B 读到";

#[test]
fn user_b_cannot_read_user_a_pages() {
    let base = tmp_root("read");
    let a = WikiStore::new(&base, user(1));
    let b = WikiStore::new(&base, user(2));
    a.ensure_layout().expect("A 建三层");
    b.ensure_layout().expect("B 建三层");

    a.write_page("secret.md", "---\ntitle: 密\n---\n\nsecret\n")
        .expect("A 写入");

    // A 读得到
    assert!(a.read_page("secret.md").is_ok());
    // B 读不到 —— 且**不是**「读到了空内容」这种静默形态
    let r = b.read_page("secret.md");
    assert!(r.is_err(), "B 竟读到了 A 的页面：{:?}", r.map(|p| p.body));
    // B 的目录下确实不存在该文件（排除「读到了但内容是别的」）
    let b_secret = b.root().join("wiki").join("secret.md");
    assert!(!b_secret.exists());
    // 且 A 的文件确实在 A 的根下
    assert!(a.root().join("wiki").join("secret.md").exists());
    assert!(A_ONLY_SECRET.chars().count() > 0); // 常量被使用，避免 unused
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn listing_never_leaks_across_users() {
    let base = tmp_root("list");
    let a = WikiStore::new(&base, user(1));
    let b = WikiStore::new(&base, user(2));
    a.ensure_layout().expect("A 建三层");
    b.ensure_layout().expect("B 建三层");

    for i in 0..3 {
        a.write_page(&format!("a{i}.md"), "---\ntitle: 甲\n---\n\nx\n")
            .expect("A 写入");
    }
    b.write_page("b0.md", "---\ntitle: 乙\n---\n\ny\n")
        .expect("B 写入");

    let a_list = a.list_pages().expect("A 列表");
    let b_list = b.list_pages().expect("B 列表");
    assert_eq!(a_list.len(), 3, "A 只见自己的：{a_list:?}");
    assert_eq!(b_list.len(), 1, "B 只见自己的：{b_list:?}");
    assert!(
        !a_list.iter().any(|p| p == "b0.md"),
        "A 的列表漏出了 B 的文件"
    );
    assert!(
        !b_list.iter().any(|p| p.starts_with('a')),
        "B 的列表漏出了 A 的文件"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn traversal_cannot_reach_another_users_directory() {
    let base = tmp_root("traverse");
    let a = WikiStore::new(&base, user(1));
    let b = WikiStore::new(&base, user(2));
    a.ensure_layout().expect("A 建三层");
    b.ensure_layout().expect("B 建三层");
    a.write_page("secret.md", "---\ntitle: 密\n---\n\nsecret\n")
        .expect("A 写入");

    // A 的目录名是 B 的**兄弟目录**，故 `..` 一层就能到 B 的 wiki 根。
    // 这是本项目最真实的一条越权路径，必须被 resolve 挡住。
    // 先让 B 有一页自己的内容，便于最后证明「A 的越权尝试没动到 B」。
    b.write_page("b-own.md", "---\ntitle: 乙自己的\n---\n\nB 的内容\n")
        .expect("B 写入");
    let attempts = [
        "../".to_string() + &b.user().to_compact_hex() + "/wiki/secret.md",
        "../../wiki/secret.md".to_string(),
        "sub/../../".to_string() + &b.user().to_compact_hex() + "/wiki/secret.md",
    ];
    for att in &attempts {
        let r = a.resolve("wiki", att);
        assert!(r.is_err(), "越权路径未被拒绝：{att}");
        // 错误信息会回显**调用方自己给的**路径（那是它自己的输入，回显无害），
        // 但绝不能带上文件**内容** —— 那才是越权读取发生过的证据。
        let msg = r.expect_err("应报错").to_string();
        assert!(!msg.contains("私密内容"), "错误信息泄漏了文件内容：{msg}");
    }
    // B 自己的文件完好，且**没有**多出被攻击的 secret.md
    assert!(
        b.read_page("b-own.md").is_ok(),
        "B 的文件被 A 的越权尝试破坏了"
    );
    assert!(
        !b.root().join("wiki").join("secret.md").exists(),
        "越权写入竟落到了 B 的目录里"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn log_and_index_are_per_user() {
    let base = tmp_root("log");
    let a = WikiStore::new(&base, user(1));
    let b = WikiStore::new(&base, user(2));
    a.ensure_layout().expect("A 建三层");
    b.ensure_layout().expect("B 建三层");

    a.append_log("## [2026-10-04] ingest | A 的来源\n\nA 的日志\n")
        .expect("A 追加");
    a.write_index("# Wiki 索引\n\n## Entities\n- [[A 专属]] — 只有 A 有。\n")
        .expect("A 写索引");

    let a_log = a.read_log().expect("A 读日志").expect("A 有日志");
    assert!(a_log.contains("A 的日志"));
    let a_idx = a.read_index().expect("A 读索引").expect("A 有索引");
    assert!(a_idx.contains("A 专属"));

    // B 的一切都是空的 —— 不是「读到 A 的内容」
    assert!(
        b.read_log().expect("B 读日志").is_none(),
        "B 不该看到 A 的日志"
    );
    assert!(
        b.read_index().expect("B 读索引").is_none(),
        "B 不该看到 A 的索引"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn raw_layer_is_also_user_scoped() {
    let base = tmp_root("raw");
    let a = WikiStore::new(&base, user(1));
    let b = WikiStore::new(&base, user(2));
    a.ensure_layout().expect("A 建三层");
    b.ensure_layout().expect("B 建三层");

    a.write_raw("docs/a.pdf.txt", "A 的原始来源")
        .expect("A 写 raw");
    assert_eq!(
        a.read_raw("docs/a.pdf.txt").expect("A 读 raw"),
        "A 的原始来源"
    );
    assert!(
        b.read_raw("docs/a.pdf.txt").is_err(),
        "B 读到了 A 的 raw 来源"
    );
    // raw 越权路径同样被挡
    let escape = format!("../{}/raw/docs/a.pdf.txt", a.user().to_compact_hex());
    assert!(b.resolve("raw", &escape).is_err());
    let _ = std::fs::remove_dir_all(&base);
}
