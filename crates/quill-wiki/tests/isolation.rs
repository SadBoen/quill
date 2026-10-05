use quill_wiki::store::WikiStore;
use quill_wiki::UserId;

fn user(n: u8) -> UserId {
    let mut b = [0u8; 16];
    b[15] = n;
    UserId::from_bytes(b)
}

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

    assert!(a.read_page("secret.md").is_ok());

    let r = b.read_page("secret.md");
    assert!(r.is_err(), "B 竟读到了 A 的页面：{:?}", r.map(|p| p.body));

    let b_secret = b.root().join("wiki").join("secret.md");
    assert!(!b_secret.exists());

    assert!(a.root().join("wiki").join("secret.md").exists());
    assert!(A_ONLY_SECRET.chars().count() > 0);
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

        let msg = r.expect_err("应报错").to_string();
        assert!(!msg.contains("私密内容"), "错误信息泄漏了文件内容：{msg}");
    }

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

    let escape = format!("../{}/raw/docs/a.pdf.txt", a.user().to_compact_hex());
    assert!(b.resolve("raw", &escape).is_err());
    let _ = std::fs::remove_dir_all(&base);
}
