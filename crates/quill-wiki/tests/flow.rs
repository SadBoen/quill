
use std::path::{Path, PathBuf};

use quill_adapters::{
    AdapterError, IndexReceipt, IngestContext, KnowledgeBackend, KnowledgePage, LintContext,
    QueryAnswer, QueryContext,
};
use quill_wiki::backend::{lint_context, page_from_wire};
use quill_wiki::index::WikiIndex;
use quill_wiki::log::parse_log;
use quill_wiki::store::WikiStore;
use quill_wiki::{run_lint, Date, LintConfig, UserId};

const SUMMARY: &str = "测试摄入";

struct FakeBackend {

    base: PathBuf,
    user: UserId,

    writes: Vec<KnowledgePage>,

    answer: QueryAnswer,

    lie_about: Option<Vec<String>>,
    seen_questions: std::sync::Mutex<Vec<String>>,

    seen_sources: std::sync::Mutex<Vec<String>>,

    seen_source_text: std::sync::Mutex<Vec<String>>,

    seen_lint_reports: std::sync::Mutex<Vec<String>>,
}

impl FakeBackend {
    fn new(base: &Path, user: UserId, writes: Vec<KnowledgePage>) -> Self {
        Self {
            base: base.to_path_buf(),
            user,
            writes,
            answer: QueryAnswer::default(),
            lie_about: None,
            seen_questions: std::sync::Mutex::new(Vec::new()),
            seen_sources: std::sync::Mutex::new(Vec::new()),
            seen_source_text: std::sync::Mutex::new(Vec::new()),
            seen_lint_reports: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn liar(base: &Path, user: UserId, claimed: &[&str]) -> Self {
        let mut b = Self::new(base, user, Vec::new());
        b.lie_about = Some(claimed.iter().map(|s| (*s).to_string()).collect());
        b
    }

    fn do_plan_ingest(&self, ctx: IngestContext) -> Result<IndexReceipt, AdapterError> {
        self.seen_sources
            .lock()
            .expect("锁")
            .push(ctx.source.rel_path);
        self.seen_source_text
            .lock()
            .expect("锁")
            .push(ctx.source.text);

        if let Some(claimed) = &self.lie_about {

            return Ok(IndexReceipt {
                touched: claimed.clone(),
                summary: "谎报回执".to_string(),
            });
        }

        let store = WikiStore::new(&self.base, self.user);
        let mut touched = Vec::new();
        for w in &self.writes {

            let page = page_from_wire(w)?;
            store.write_page(&page.path, &w.content)?;
            touched.push(page.path);
        }
        Ok(IndexReceipt {
            touched,
            summary: SUMMARY.to_string(),
        })
    }
}

impl KnowledgeBackend for FakeBackend {
    fn plan_ingest(
        &self,
        _user: UserId,
        ctx: IngestContext,
    ) -> impl std::future::Future<Output = Result<IndexReceipt, AdapterError>> + Send {
        std::future::ready(self.do_plan_ingest(ctx))
    }

    fn answer_query(
        &self,
        _user: UserId,
        ctx: QueryContext,
    ) -> impl std::future::Future<Output = Result<QueryAnswer, AdapterError>> + Send {
        self.seen_questions.lock().expect("锁").push(ctx.question);
        std::future::ready(Ok(self.answer.clone()))
    }

    fn lint_semantics(
        &self,
        _user: UserId,
        ctx: LintContext,
    ) -> impl std::future::Future<Output = Result<Option<Vec<String>>, AdapterError>> + Send {
        self.seen_lint_reports
            .lock()
            .expect("锁")
            .push(ctx.report_text);
        std::future::ready(Ok(None))
    }
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::Poll;

    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,

            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

fn user(n: u8) -> UserId {
    let mut b = [0u8; 16];
    b[15] = n;
    UserId::from_bytes(b)
}

fn tmp_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "quill-wiki-flow-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("建临时根");
    p
}

fn day() -> Date {
    Date::parse("2026-10-04").expect("测试日期")
}

fn pages_with(paths: &[&str]) -> Vec<KnowledgePage> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| KnowledgePage {
            rel_path: (*p).to_string(),
            content: format!(
                "---\ntitle: 页{i}\ntype: concept\ntags: [测试]\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n摘要{i}。\n\n见 [[页0]]\n"
            ),
        })
        .collect()
}

#[test]
fn ingest_writes_pages_index_and_log() {
    let base = tmp_root("ingest-ok");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store
        .write_raw("papers/a.pdf.txt", "这是来源正文")
        .expect("写来源");

    let backend = FakeBackend::new(
        &base,
        user(1),
        pages_with(&["concepts/页0.md", "concepts/页1.md"]),
    );
    let req = quill_wiki::IngestRequest {
        source_rel: "papers/a.pdf.txt".to_string(),
        date: day(),
        focus: vec![],
    };

    let out =
        block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("摄入成功");

    assert_eq!(out.written.len(), 2);
    let p0 = store.read_page("concepts/页0.md").expect("读回页0");
    assert_eq!(p0.frontmatter.title.as_deref(), Some("页0"));

    let idx_text = store.read_index().expect("读索引").expect("索引存在");
    let idx = WikiIndex::parse(&idx_text).expect("索引可解析");
    assert_eq!(idx.len(), 2, "索引应有两条：\n{idx_text}");

    let log_text = store.read_log().expect("读日志").expect("日志存在");
    assert_eq!(
        log_text.lines().filter(|l| l.starts_with("## [")).count(),
        1,
        "应恰好一条标题行：\n{log_text}"
    );
    let parsed = parse_log(&log_text);
    assert_eq!(parsed.skipped_headings, 0);
    assert_eq!(parsed.entries[0].op, quill_wiki::LogOp::Ingest);
    assert!(parsed.entries[0].title.contains("a.pdf.txt"));

    assert!(
        parsed.entries[0].body.contains(SUMMARY),
        "摘要未落进日志：\n{log_text}"
    );

    assert_eq!(
        backend.seen_sources.lock().expect("锁").as_slice(),
        &["papers/a.pdf.txt".to_string()]
    );
    assert_eq!(
        backend.seen_source_text.lock().expect("锁").as_slice(),
        &["这是来源正文".to_string()]
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ingest_is_idempotent_on_repeat() {
    let base = tmp_root("ingest-idem");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");
    let backend = FakeBackend::new(&base, user(1), pages_with(&["concepts/页0.md"]));
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };

    block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("第一次");
    let out2 =
        block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("第二次");

    assert_eq!(out2.index_entries, 1, "重复摄入不该让索引膨胀");
    let idx = WikiIndex::parse(&store.read_index().expect("读").expect("有")).expect("解析");
    assert_eq!(idx.len(), 1);

    let log = store.read_log().expect("读").expect("有");
    assert_eq!(log.lines().filter(|l| l.starts_with("## [")).count(), 2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ingest_rejects_escape_paths_from_the_llm_and_writes_nothing() {
    let base = tmp_root("ingest-escape");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");

    let mut pages = pages_with(&["concepts/good.md"]);
    pages.push(KnowledgePage {
        rel_path: format!("../{}/wiki/stolen.md", user(2).to_compact_hex()),
        content: "---\ntitle: 偷\ntype: concept\n---\n\n越权内容\n".to_string(),
    });
    let backend = FakeBackend::new(&base, user(1), pages);
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };

    let r = block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req));
    assert!(r.is_err(), "越权路径竟被接受了：{:?}", r.map(|o| o.written));

    let victim = WikiStore::new(&base, user(2))
        .root()
        .join("wiki")
        .join("stolen.md");
    assert!(!victim.exists(), "越权写入竟然落盘了：{}", victim.display());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ingest_accepts_ordinary_relative_paths() {
    let base = tmp_root("ingest-legal");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");
    let backend = FakeBackend::new(
        &base,
        user(1),
        pages_with(&["a.md", "deep/nested/dir/b.md", "含中文 名.md"]),
    );
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };
    let out =
        block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("应放行");
    assert_eq!(out.written.len(), 3);
    for rel in &out.written {
        assert!(store.read_page(rel).is_ok(), "合法路径 {rel} 未落盘");
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ingest_rejects_a_lying_receipt_and_writes_no_log_entry() {
    let base = tmp_root("ingest-liar");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");

    let backend = FakeBackend::liar(
        &base,
        user(1),
        &["concepts/根本没写.md", "concepts/也没有.md"],
    );
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };

    let r = block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req));
    assert!(r.is_err(), "谎报回执竟被采信了：{:?}", r.map(|o| o.written));

    let msg = r.expect_err("应报错").to_string();
    assert!(
        msg.contains("没写") || msg.contains("读不回来"),
        "错误文案没说明回执与磁盘不一致：{msg}"
    );

    for rel in ["concepts/根本没写.md", "concepts/也没有.md"] {
        assert!(
            store.read_page(rel).is_err(),
            "{rel} 不该存在（backend 根本没写）"
        );
    }

    match store.read_log().expect("读日志") {
        None => {}
        Some(log) => {
            let parsed = parse_log(&log);
            assert!(
                !parsed
                    .entries
                    .iter()
                    .any(|e| e.op == quill_wiki::LogOp::Ingest),
                "失败的 ingest 不得留下成功日志：\n{log}"
            );
        }
    }

    match store.read_index().expect("读索引") {
        None => {}
        Some(text) => {
            let idx = WikiIndex::parse(&text).expect("解析");
            assert!(idx.is_empty(), "失败的摄入不得产出索引条目：\n{text}");
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn query_reads_index_then_answers_and_logs() {
    let base = tmp_root("query");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");

    let backend = FakeBackend::new(&base, user(1), pages_with(&["concepts/页0.md"]));
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };
    block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("摄入");

    let mut qb = FakeBackend::new(&base, user(1), Vec::new());
    qb.answer = QueryAnswer {
        answer: "答案正文".to_string(),
        citations: vec!["页0".to_string()],
        archival_candidate: Some(KnowledgePage {
            rel_path: "synthesis/新页.md".to_string(),
            content: "---\ntitle: 新页\ntype: synthesis\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n新页正文。\n".to_string(),
        }),
    };

    let q = quill_wiki::QueryRequest {
        question: "页0 是什么".to_string(),
        date: day(),
    };
    let out = block_on(quill_wiki::query::query(&store, &qb, user(1), &q)).expect("查询成功");

    assert_eq!(out.answer, "答案正文");
    assert!(!out.used_pages.is_empty(), "查询应至少用到一页：{out:?}");
    assert!(out.archival_candidate.is_some(), "归档建议应被带回");

    let log = store.read_log().expect("读").expect("有");
    let parsed = parse_log(&log);
    assert_eq!(parsed.skipped_headings, 0);
    let ops: Vec<quill_wiki::LogOp> = parsed.entries.iter().map(|e| e.op).collect();
    assert!(ops.contains(&quill_wiki::LogOp::Ingest));
    assert!(ops.contains(&quill_wiki::LogOp::Query));

    assert!(
        store.read_page("synthesis/新页.md").is_err(),
        "归档不该自动落盘（必须由人/策略确认）"
    );

    let arc = quill_wiki::query::archive_answer(
        &store,
        out.archival_candidate.as_ref().expect("归档建议"),
        day(),
    )
    .expect("归档成功");
    assert_eq!(arc.path, "synthesis/新页.md");
    assert!(store.read_page("synthesis/新页.md").is_ok());

    let idx = WikiIndex::parse(&store.read_index().expect("读").expect("有")).expect("解析");
    assert!(
        idx.contains("新页"),
        "归档后索引未更新：{:?}",
        idx.get("新页")
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn archive_answer_rejects_an_unparsable_candidate() {
    let base = tmp_root("archive-bad");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");

    let bad = KnowledgePage {
        rel_path: "synthesis/坏页.md".to_string(),

        content: "# 只有标题没有 frontmatter\n".to_string(),
    };
    let r = quill_wiki::query::archive_answer(&store, &bad, day());
    assert!(r.is_err(), "不合法页面竟被归档了：{:?}", r.map(|o| o.path));
    assert!(
        store.read_page("synthesis/坏页.md").is_err(),
        "不合法页面不该落盘（兜底成空页面 = 资料库被无声污染）"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn lint_report_crosses_the_boundary_as_text() {
    let base = tmp_root("lint-wire");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store
        .write_page(
            "lonely.md",
            "---\ntitle: 孤儿\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n见 [[不存在的页]]\n",
        )
        .expect("写页");

    let report = run_lint(&store, day(), &LintConfig::default()).expect("lint 成功");
    let pages = store.load_all_pages().expect("读全部页");

    let backend = FakeBackend::new(&base, user(1), Vec::new());
    let ctx = lint_context(report.render(), &pages);

    assert!(
        ctx.pages
            .iter()
            .any(|p| p.rel_path == "lonely.md" && p.content.contains("孤儿")),
        "页面未跨界：{:?}",
        ctx.pages
    );
    let out = block_on(backend.lint_semantics(user(1), ctx)).expect("语义体检调用成功");
    assert!(out.is_none(), "本假 LLM 一律回答「无需补充」");

    let seen = backend.seen_lint_reports.lock().expect("锁").clone();
    assert_eq!(seen.len(), 1);

    assert!(
        seen[0].contains("已检查"),
        "报告文本缺覆盖统计：{}",
        seen[0]
    );
    assert!(
        seen[0].contains("orphan-page"),
        "报告文本缺规则 ID：{}",
        seen[0]
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn lint_run_reports_and_logs() {
    let base = tmp_root("lint");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");

    store
        .write_page("lonely.md", "---\ntitle: 孤儿\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n见 [[不存在的页]]\n")
        .expect("写页");

    let report = run_lint(&store, day(), &LintConfig::default()).expect("lint 成功");
    assert!(report.has_problems());
    assert_eq!(report.count(quill_wiki::Rule::OrphanPage), 1);
    assert_eq!(report.count(quill_wiki::Rule::BrokenLink), 1);
    assert_eq!(report.stats.pages_checked, 1);

    let log = store.read_log().expect("读").expect("有");
    let parsed = parse_log(&log);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].op, quill_wiki::LogOp::Lint);
    assert!(parsed.entries[0].body.contains("已检查"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn lint_does_not_run_on_every_ingest() {

    let base = tmp_root("no-auto-lint");
    let store = WikiStore::new(&base, user(1));
    store.ensure_layout().expect("建三层");
    store.write_raw("a.txt", "来源").expect("写来源");
    let backend = FakeBackend::new(&base, user(1), pages_with(&["concepts/页0.md"]));
    let req = quill_wiki::IngestRequest {
        source_rel: "a.txt".to_string(),
        date: day(),
        focus: vec![],
    };
    block_on(quill_wiki::ingest::ingest(&store, &backend, user(1), &req)).expect("摄入");
    let log = store.read_log().expect("读").expect("有");
    let parsed = parse_log(&log);
    assert!(
        !parsed
            .entries
            .iter()
            .any(|e| e.op == quill_wiki::LogOp::Lint),
        "ingest 不得隐式触发 lint：{log}"
    );
    let _ = std::fs::remove_dir_all(&base);
}
