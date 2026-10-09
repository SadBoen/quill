//! `GET /api/experts/export` + `POST /api/experts/import` 的往返契约（Q031 / Q032）。
//!
//! 判据是「导入可往返、与 import 对称」，这里把它钉成机器可跑的四件事：
//!
//! 1. **逐字段等价**：A 导出 → 导入到 B（跨账号）或覆盖导入回 A（同账号）→
//!    再导出，两次的 `experts` 数组**每个字段都相同**。导出刻意不带
//!    owner / 指纹 / 时间戳，所以比较时**没有任何字段需要排除** ——
//!    这一点由 `export_carries_only_the_7_bundle_fields` 单独钉住。
//! 2. **不是全量替换**：清单里没有的专家在导入后原样还在。
//! 3. **逐条结果**：created / updated / skipped / failed + 原因；一条坏不影响
//!    其余，failed > 0 时 `ok=false`。
//! 4. **版本不匹配直接拒**，且导出产出的完整形状（含 `redacted` / `note`）
//!    能原样喂回导入。
//!
//! 另外钉住：内置系统专家不导出也不被动到；软删专家不导出、可由导入复活。

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_agent::{Expert, ExpertRepository, SYSTEM_OWNER};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::experts_repo::SqlxExpertRepository;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const UID_B: &str = "0192b7c8-0000-7000-8000-000000000002";
const TOKEN_A: &str = "tok-a";
const TOKEN_B: &str = "tok-b";

/// 导出清单里一条专家的全部字段。这里刻意手抄一份（而不是引用实现里的常量）：
/// 集成测试钉住的是**线上形状**，实现改了常量而没改行为时这里必须一起红。
const BUNDLE_FIELDS: [&str; 7] = [
    "id",
    "display_name",
    "description",
    "instructions",
    "model",
    "source_template",
    "default_enabled",
];

fn uid(raw: &str) -> quill_domain::UserId {
    quill_domain::UserId::parse(raw).expect("测试 UID 必须合法")
}

fn expert_id(raw: &str) -> quill_adapters::ExpertId {
    quill_adapters::ExpertId::parse(raw).expect("测试专家 id 必须合法")
}

struct Harness {
    db: TestDb,
}

impl Harness {
    fn new(label: &str) -> Self {
        Self {
            db: TestDb::new(label),
        }
    }

    fn state(&self) -> AppState {
        let resolver = EnvTokenResolver::new(vec![
            (
                TOKEN_A.to_string(),
                AuthContext {
                    user_id: uid(UID_A),
                    is_admin: true,
                },
            ),
            (
                TOKEN_B.to_string(),
                AuthContext {
                    user_id: uid(UID_B),
                    is_admin: false,
                },
            ),
        ]);
        AppState {
            config: Config::from_env(),
            tokens: Arc::new(resolver),
            db: Some(self.db.bridge()),
            db_problem: None,
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            member_control: Default::default(),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        }
    }
}

fn req(
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    b.body(match body {
        Some(v) => Body::from(v.to_string()),
        None => Body::empty(),
    })
    .expect("构造请求失败")
}

async fn send(h: &Harness, r: Request<Body>) -> (StatusCode, String) {
    let resp = build_router(h.state()).oneshot(r).await.expect("请求失败");
    let st = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("读响应体")
        .to_bytes();
    (
        st,
        String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8"),
    )
}

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

async fn export(h: &Harness, token: &str) -> serde_json::Value {
    let (st, text) = send(h, req("GET", "/api/experts/export", Some(token), None)).await;
    assert_eq!(st, StatusCode::OK, "导出应当 200：{text}");
    json_of(&text)
}

async fn import(
    h: &Harness,
    token: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let (st, text) = send(
        h,
        req("POST", "/api/experts/import", Some(token), Some(body)),
    )
    .await;
    (st, json_of(&text))
}

async fn create(h: &Harness, token: &str, body: serde_json::Value) -> serde_json::Value {
    let (st, text) = send(
        h,
        req("POST", "/api/experts", Some(token), Some(body.clone())),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "建专家应当 201（{body}）：{text}");
    json_of(&text)
}

async fn patch(h: &Harness, token: &str, id: &str, body: serde_json::Value) -> serde_json::Value {
    let path = format!("/api/experts/{id}");
    let (st, text) = send(h, req("PATCH", &path, Some(token), Some(body.clone()))).await;
    assert_eq!(st, StatusCode::OK, "改专家应当 200（{body}）：{text}");
    json_of(&text)
}

async fn delete(h: &Harness, token: &str, id: &str) -> serde_json::Value {
    let path = format!("/api/experts/{id}");
    let (st, text) = send(h, req("DELETE", &path, Some(token), None)).await;
    assert_eq!(st, StatusCode::OK, "删专家应当 200：{text}");
    json_of(&text)
}

fn items(bundle: &serde_json::Value) -> &Vec<serde_json::Value> {
    bundle["experts"]
        .as_array()
        .unwrap_or_else(|| panic!("清单必须带 experts 数组：{bundle}"))
}

fn ids(bundle: &serde_json::Value) -> Vec<String> {
    items(bundle)
        .iter()
        .map(|it| {
            it["id"]
                .as_str()
                .unwrap_or_else(|| panic!("条目必须带 id：{it}"))
                .to_string()
        })
        .collect()
}

fn by_id<'a>(bundle: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    items(bundle)
        .iter()
        .find(|it| it["id"] == id)
        .unwrap_or_else(|| panic!("清单里没有 {id}：{bundle}"))
}

fn summary(resp: &serde_json::Value, key: &str) -> u64 {
    resp["summary"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("summary 缺 {key}：{resp}"))
}

/// 铺一个内置系统专家（属主 = SYSTEM_OWNER，is_builtin=1）。走的是
/// `create_builtin_expert` 同一条仓库写入路径。
fn seed_builtin(h: &Harness, raw_id: &str) {
    let repo = SqlxExpertRepository::new(h.db.bridge());
    let e = Expert::builtin_with_source(
        expert_id(raw_id),
        "内置助手",
        "随版本发布的内置专家",
        "内置人格正文",
        None,
        None,
    )
    .expect("构造内置专家");
    repo.put(&e).expect("铺内置专家失败");
    assert_eq!(e.owner(), SYSTEM_OWNER, "内置专家的属主必须是全零用户");
}

// ---------------------------------------------------------------- 往返（判据）

#[tokio::test]
async fn export_import_round_trip_is_field_identical_across_and_within_accounts() {
    let h = Harness::new("expert-bundle-round-trip");

    // A 建两个专家：一个把六个可搬字段全部填满；一个把可空字段留空、
    // 带**首尾空白**的 description、default_enabled=false —— 这些正是
    // 「少读/少写一个字段」最容易漏掉的地方。
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "cost-analyst",
            "display_name": "成本分析师",
            "description": "算清成本口径",
            "instructions": "先问口径再算，逐项列明数据来源。",
            "model": "qwen3.5",
            "source_template": "ai-coding-coach",
        }),
    )
    .await;
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "scribe",
            "display_name": "记录员",
            "description": "记录会议要点",
            "instructions": "",
        }),
    )
    .await;
    // POST 的 description 走 need_str（会 trim），留白只能靠 PATCH 写入 ——
    // 正好用来钉住「导入端不得 trim」。
    patch(
        &h,
        TOKEN_A,
        "scribe",
        serde_json::json!({ "description": "  首尾留白  ", "default_enabled": false }),
    )
    .await;

    let b1 = export(&h, TOKEN_A).await;
    assert_eq!(b1["bundle_version"], 1, "版本字段必须在：{b1}");
    assert_eq!(items(&b1).len(), 2, "A 应当恰好导出两条：{b1}");
    for it in items(&b1) {
        let keys: BTreeSet<&str> = it
            .as_object()
            .expect("条目必须是对象")
            .keys()
            .map(String::as_str)
            .collect();
        let want: BTreeSet<&str> = BUNDLE_FIELDS.iter().copied().collect();
        assert_eq!(
            keys, want,
            "每条恰好这 7 个字段（内部列不外流、字段不静默缺失）：{it}"
        );
    }
    assert_eq!(
        by_id(&b1, "scribe")["description"],
        "  首尾留白  ",
        "首尾空白必须原样导出：{b1}"
    );
    assert_eq!(by_id(&b1, "scribe")["default_enabled"], false, "{b1}");
    assert_eq!(by_id(&b1, "cost-analyst")["model"], "qwen3.5", "{b1}");

    // —— 跨账号：A 的清单导进 B ——
    let (st, r) = import(&h, TOKEN_B, b1.clone()).await;
    assert_eq!(st, StatusCode::OK, "导入应当 200：{r}");
    assert_eq!(r["ok"], true, "无失败项时 ok=true：{r}");
    assert_eq!(summary(&r, "created"), 2, "{r}");
    assert_eq!(summary(&r, "failed"), 0, "{r}");

    let b2 = export(&h, TOKEN_B).await;
    assert_eq!(
        b2["experts"], b1["experts"],
        "跨账号往返必须逐字段等价：\nA={b1}\nB={b2}"
    );
    assert_eq!(
        items(&b2).len(),
        2,
        "B 只应有导入进来的两条（没有别的专家被凭空带出）：{b2}"
    );

    // —— 同账号覆盖导入：先把 A 的库内状态改坏，再用 b1 覆盖回去 ——
    patch(
        &h,
        TOKEN_A,
        "cost-analyst",
        serde_json::json!({
            "display_name": "改坏的名字",
            "model": "other-model",
            "default_enabled": false,
        }),
    )
    .await;
    let (st, r) = import(&h, TOKEN_A, b1.clone()).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(summary(&r, "updated"), 1, "被改坏的那条要被覆盖回去：{r}");
    assert_eq!(
        summary(&r, "skipped"),
        1,
        "没动过的那条应判 skipped（不白写 updated_at）：{r}"
    );
    assert_eq!(summary(&r, "created"), 0, "{r}");
    let b3 = export(&h, TOKEN_A).await;
    assert_eq!(
        b3["experts"], b1["experts"],
        "同账号覆盖导入后必须与原始清单逐字段等价：{b3}"
    );
}

#[tokio::test]
async fn export_carries_only_the_7_bundle_fields_so_nothing_needs_excluding() {
    let h = Harness::new("expert-bundle-shape");
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "mine",
            "display_name": "我的专家",
            "description": "描述",
            "instructions": "人格",
            "model": "m1",
            "source_template": "ai-coding-coach",
        }),
    )
    .await;
    let b = export(&h, TOKEN_A).await;
    let it = by_id(&b, "mine");
    // 指纹 / 归属 / 不变量 / 时间戳一律不出现在条目里（不是「靠比较时排除」）。
    let text = it.to_string();
    for forbidden in [
        "asset_hash",
        "persona_hash",
        "owner_user_id",
        "visibility",
        "is_builtin",
        "deleted_at",
        "created_at",
        "updated_at",
    ] {
        assert!(
            !text.contains(forbidden),
            "条目泄漏了内部列 {forbidden}：{text}"
        );
    }
    // redacted 如实点名剔掉了什么。
    let redacted = b["redacted"]
        .as_array()
        .expect("必须带 redacted 清单")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    for want in ["asset_hash", "persona_hash", "owner_user_id"] {
        assert!(
            redacted.iter().any(|r| r.ends_with(want)),
            "redacted 要点名 {want}：{redacted:?}"
        );
    }
    assert!(
        b["note"].as_str().is_some_and(|s| !s.is_empty()),
        "note 必须说明口径：{b}"
    );
}

// ---------------------------------------------------------------- 语义与逐条结果

#[tokio::test]
async fn import_never_touches_experts_that_are_not_in_the_bundle() {
    let h = Harness::new("expert-bundle-no-replace");
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "keeper",
            "display_name": "要搬走的",
            "description": "d",
            "instructions": "i",
        }),
    )
    .await;
    let b1 = export(&h, TOKEN_A).await;
    assert_eq!(ids(&b1), vec!["keeper"], "{b1}");

    create(
        &h,
        TOKEN_B,
        serde_json::json!({
            "id": "bystander",
            "display_name": "没被点名的",
            "description": "d",
            "instructions": "i",
            "model": "m2",
        }),
    )
    .await;
    let before = export(&h, TOKEN_B).await;
    assert_eq!(ids(&before), vec!["bystander"], "{before}");

    // 空清单：什么都不写，也不删。
    let (st, r) = import(
        &h,
        TOKEN_B,
        serde_json::json!({ "bundle_version": 1, "experts": [] }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(summary(&r, "total"), 0, "{r}");
    let after_empty = export(&h, TOKEN_B).await;
    assert_eq!(
        after_empty["experts"], before["experts"],
        "空清单导入不得动到任何专家：{after_empty}"
    );

    // 只含 keeper 的清单：bystander 仍然原样在。
    let (st, r) = import(
        &h,
        TOKEN_B,
        serde_json::json!({ "bundle_version": 1, "experts": b1["experts"].clone() }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(summary(&r, "created"), 1, "{r}");
    let after = export(&h, TOKEN_B).await;
    assert_eq!(ids(&after), vec!["bystander", "keeper"], "{after}");
    assert_eq!(
        by_id(&after, "bystander"),
        by_id(&before, "bystander"),
        "清单里没有的专家必须逐字段原样保留：{after}"
    );
}

#[tokio::test]
async fn import_reports_every_item_and_one_bad_item_does_not_sink_the_batch() {
    let h = Harness::new("expert-bundle-per-item");
    let good = serde_json::json!({
        "id": "alpha",
        "display_name": "阿尔法",
        "description": "",
        "instructions": "",
        "model": null,
        "source_template": null,
        "default_enabled": true,
    });
    let bundle = serde_json::json!({
        "bundle_version": 1,
        "experts": [
            good.clone(),
            // 1) 非法 id（大写 + 空格）
            {
                "id": "Bad Id", "display_name": "坏 id", "description": "",
                "instructions": "", "model": null, "source_template": null,
                "default_enabled": true,
            },
            // 2) 缺 model 键（不允许省略键）
            {
                "id": "no-model-key", "display_name": "缺字段", "description": "",
                "instructions": "", "source_template": null, "default_enabled": true,
            },
            // 3) 同一清单里 id 重复
            good.clone(),
            // 4) 多余字段
            {
                "id": "extra-key", "display_name": "多字段", "description": "",
                "instructions": "", "model": null, "source_template": null,
                "default_enabled": true, "unexpected": 1,
            },
            // 5) 类型错：model 是数字
            {
                "id": "echo", "display_name": "类型错", "description": "",
                "instructions": "", "model": 7, "source_template": null,
                "default_enabled": true,
            },
        ],
    });

    let (st, r) = import(&h, TOKEN_A, bundle).await;
    assert_eq!(st, StatusCode::OK, "批处理本身成功：{r}");
    assert_eq!(
        r["ok"], false,
        "有失败项必须 ok=false（不许静默部分成功）：{r}"
    );
    assert_eq!(summary(&r, "total"), 6, "{r}");
    assert_eq!(summary(&r, "created"), 1, "{r}");
    assert_eq!(summary(&r, "failed"), 5, "{r}");
    let results = r["results"].as_array().expect("results 必须是数组");
    assert_eq!(results.len(), 6, "每条都要有结果：{r}");
    for res in results {
        assert!(
            res["reason"].as_str().is_some_and(|s| !s.is_empty()),
            "每条都要给原因：{res}"
        );
        assert!(
            !res["status"].as_str().unwrap_or("").is_empty(),
            "每条都要给状态：{res}"
        );
    }
    assert_eq!(results[0]["status"], "created", "{r}");
    assert_eq!(results[1]["status"], "failed", "非法 id 必须逐条判红：{r}");
    assert!(
        results[1]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("Bad Id"),
        "原因要点名是哪个 id：{r}"
    );
    assert_eq!(results[2]["status"], "failed", "{r}");
    assert!(
        results[2]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("model"),
        "原因要点名缺哪个字段：{r}"
    );
    assert!(
        results[3]["reason"].as_str().unwrap_or("").contains("重复"),
        "重复 id 要说清楚：{r}"
    );
    assert!(
        results[4]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("unexpected"),
        "多余字段要被点名：{r}"
    );
    assert_eq!(results[5]["status"], "failed", "{r}");

    // 唯一成功的那条**真的落库**（不许假成功）；失败的**一条都没落**。
    let b = export(&h, TOKEN_A).await;
    assert_eq!(ids(&b), vec!["alpha"], "只有 alpha 该落库：{b}");

    // 重导一次：这次是 skipped（逐字段相同，不白写 updated_at）。
    let (st, r2) = import(
        &h,
        TOKEN_A,
        serde_json::json!({ "bundle_version": 1, "experts": [good] }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r2}");
    assert_eq!(summary(&r2, "skipped"), 1, "{r2}");
    assert_eq!(summary(&r2, "updated"), 0, "{r2}");
    assert_eq!(r2["ok"], true, "{r2}");
}

#[tokio::test]
async fn import_refuses_unknown_versions_and_shapes_but_accepts_the_export_verbatim() {
    let h = Harness::new("expert-bundle-guards");

    // 新账号的导出：空清单也是 200 + 空数组（不是 404 / 501）。
    let empty = export(&h, TOKEN_A).await;
    assert_eq!(items(&empty).len(), 0, "{empty}");

    for (body, needle) in [
        (
            serde_json::json!({ "bundle_version": 2, "experts": [] }),
            "bundle_version",
        ),
        (serde_json::json!({ "experts": [] }), "bundle_version"),
        (serde_json::json!({ "bundle_version": 1 }), "experts"),
        (
            serde_json::json!({ "bundle_version": 1, "experts": "nope" }),
            "experts",
        ),
        (
            serde_json::json!({ "bundle_version": 1, "experts": [], "nope": 1 }),
            "nope",
        ),
    ] {
        let (st, r) = import(&h, TOKEN_A, body.clone()).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body} 必须 400：{r}");
        assert!(
            r.to_string().contains(needle),
            "{body} 的报错要点名 {needle}：{r}"
        );
    }

    // 被拒的批不该留下任何行。
    let after = export(&h, TOKEN_A).await;
    assert_eq!(items(&after).len(), 0, "被拒的导入不得写库：{after}");

    // 导出产出的完整形状（含 redacted / note）必须能被导入原样接受 ——
    // 这就是「与 import 对称」：导出的东西能再喂回去。
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "verbatim",
            "display_name": "原样回灌",
            "description": "d",
            "instructions": "i",
            "model": null,
            "source_template": "ai-coding-coach",
        }),
    )
    .await;
    let b = export(&h, TOKEN_A).await;
    assert!(b.get("redacted").is_some(), "{b}");
    assert!(b.get("note").is_some(), "{b}");
    let (st, r) = import(&h, TOKEN_A, b.clone()).await;
    assert_eq!(st, StatusCode::OK, "导出的形状必须能被导入接受：{r}");
    assert_eq!(summary(&r, "skipped"), 1, "原样回灌 = 无改动：{r}");
    assert_eq!(r["ok"], true, "{r}");
    let b2 = export(&h, TOKEN_A).await;
    assert_eq!(b2["experts"], b["experts"], "{b2}");
}

#[tokio::test]
async fn a_soft_deleted_expert_is_not_exported_and_is_revived_by_import() {
    let h = Harness::new("expert-bundle-revive");
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "phoenix",
            "display_name": "不死鸟",
            "description": "d",
            "instructions": "i",
            "model": "m3",
        }),
    )
    .await;
    let b1 = export(&h, TOKEN_A).await;
    assert_eq!(ids(&b1), vec!["phoenix"], "{b1}");

    let del = delete(&h, TOKEN_A, "phoenix").await;
    assert_eq!(del["deleted"], true, "{del}");
    let after_delete = export(&h, TOKEN_A).await;
    assert_eq!(
        items(&after_delete).len(),
        0,
        "软删行是墓碑，不进清单：{after_delete}"
    );

    // 导回删除前的清单 → 复活，且逐字段等价。
    let (st, r) = import(&h, TOKEN_A, b1.clone()).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(summary(&r, "updated"), 1, "软删行应被 upsert 复活：{r}");
    assert!(
        r["results"][0]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("恢复"),
        "要如实说明是「恢复」而不是「新建」：{r}"
    );
    let b2 = export(&h, TOKEN_A).await;
    assert_eq!(b2["experts"], b1["experts"], "{b2}");
}

// ---------------------------------------------------------------- 内置专家隔离

#[tokio::test]
async fn export_never_carries_builtins_and_import_never_touches_them() {
    let h = Harness::new("expert-bundle-builtin");
    seed_builtin(&h, "builtin-helper");
    create(
        &h,
        TOKEN_A,
        serde_json::json!({
            "id": "mine",
            "display_name": "我的",
            "description": "d",
            "instructions": "i",
        }),
    )
    .await;

    let b = export(&h, TOKEN_A).await;
    assert_eq!(ids(&b), vec!["mine"], "内置专家不得进清单：{b}");
    assert!(
        !b.to_string().contains("builtin-helper"),
        "清单里出现了内置专家：{b}"
    );

    // 清单里出现一个与内置同名的条目：只建**调用方自己的**那份，内置行不受影响
    // （可见性去重时用户自己的那份优先，这是既有口径）。
    let (st, r) = import(
        &h,
        TOKEN_A,
        serde_json::json!({
            "bundle_version": 1,
            "experts": [{
                "id": "builtin-helper",
                "display_name": "自建的同名专家",
                "description": "d",
                "instructions": "i",
                "model": null,
                "source_template": null,
                "default_enabled": true,
            }],
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(summary(&r, "created"), 1, "{r}");

    // 内置行仍在系统属主名下、原样未动。
    let repo = SqlxExpertRepository::new(h.db.bridge());
    let builtin = repo
        .get(&SYSTEM_OWNER, &expert_id("builtin-helper"))
        .expect("读内置专家失败")
        .expect("内置专家必须还在");
    assert!(builtin.is_builtin(), "内置标记不许被改写");
    assert_eq!(builtin.display_name(), "内置助手", "内置行内容不许被改写");

    // 导出只带用户自己的那份（内置的那份仍然不出现）。
    let b2 = export(&h, TOKEN_A).await;
    assert_eq!(ids(&b2), vec!["builtin-helper", "mine"], "{b2}");
    assert_eq!(
        by_id(&b2, "builtin-helper")["display_name"],
        "自建的同名专家",
        "导出的是用户自己那份：{b2}"
    );
}

#[tokio::test]
async fn bundle_routes_require_auth_before_anything_else() {
    let h = Harness::new("expert-bundle-401");
    for (method, path) in [
        ("POST", "/api/experts/import"),
        ("GET", "/api/experts/export"),
    ] {
        let (st, body) = send(&h, req(method, path, None, None)).await;
        assert_eq!(
            st,
            StatusCode::UNAUTHORIZED,
            "{method} {path} 必须 401：{body}"
        );
    }
}
