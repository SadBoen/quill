//! 生产用的成员执行器：让一次派工**真的跑起来**（M2「专家与专家团真执行」）。
//!
//! 与 `quill-agent` 预留接口的配合：`Dispatcher::dispatch_round` 是**同步**的，
//! 内部用空 waker 自旋 `poll` 掉 `MemberExecutor::start` 返回的 future
//! （`crates/quill-agent/src/dispatch.rs:764`，注释写着「真实执行器需接 tokio
//! （quill-server 侧注入）」）。空 waker 意味着：**返回一个「需要外部唤醒」的
//! future 会空转到 panic**。所以这里的做法是 —— 在 `start` 里就把活干完，
//! 返回一个已经 Ready 的 future。
//!
//! 「把 async 调用跑成同步结果」用的是一条**独立线程 + 独立 current-thread
//! 运行时**，理由与 `crate::mcp_client::call_tool_blocking` 里那段注释完全相同
//! （直接沿用，不另发明）：
//!
//! - `Handle::block_on` 在 async 上下文里会 panic（tokio 不许嵌套驱动），
//!   `#[tokio::test]` 默认的 current-thread 运行时也会当场炸；
//! - `block_in_place` + `block_on` 在 current-thread 运行时上同样 panic；
//! - 自己开线程与运行时则与外面是什么 flavor 无关。多付一次线程创建（微秒级）。
//!
//! 代价说清楚：调用方那个线程会**阻塞**整轮（成员数 × 模型延迟）。所以
//! **`dispatch_round` 必须在阻塞上下文里调用**（本仓库走 `spawn_blocking`），
//! 否则会占住一个 tokio worker。

use std::sync::Arc;

use quill_adapters::{
    AbortScope, AdapterError, ExpertId, MemberExecutor, MemberId, MemberOutcome,
    MemberStartRequest, Message, UserId,
};
use quill_provider::{Message as LlmMessage, SharedProvider};

use crate::db::{blob_of, storage_error, DbBridge};

/// 子 agent 的固定前缀指令。
///
/// 三件事必须说清（都是子 agent 的结构决定的，不是文风）：
/// 它是**被委派的成员**、产出会被主持人汇总、**期间没有交互通道** ——
/// 子 agent 没有人可以回答问题，所以它不该提问，只能给出可用的结论。
const DELEGATION_PREAMBLE: &str = "\
你是一个被委派的团队成员，正在独立完成主持人交给你的一个子任务。\
你的产出会被汇总给主持人，期间没有人与你交互，所以不要提问、不要请求确认 —— \
直接给出可用的结论。信息不足时，用你已有的知识做出最合理的判断，并标出不确定之处。";

/// 用 provider 真跑一次成员任务的执行器。
///
/// 无状态：每次 `start` 都独立拼 prompt 并调一次模型。成员的会话归属由
/// `DispatchScope` 在台账层记录，这里不落会话 —— 见 `crate::dispatch_ledger`。
pub struct ProviderMemberExecutor {
    provider: SharedProvider,
    /// 与聊天同一处取请求参数（模型名、max_tokens 等）—— 走 `crate::llm::build_request`，
    /// 避免派工这条线自己拼一套、与聊天那条线口径分叉。
    llm_config: crate::llm::LlmConfig,
    db: Arc<DbBridge>,
}

impl std::fmt::Debug for ProviderMemberExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderMemberExecutor")
            .field("model", &self.llm_config.model)
            .finish_non_exhaustive()
    }
}

impl ProviderMemberExecutor {
    pub fn new(
        provider: SharedProvider,
        llm_config: crate::llm::LlmConfig,
        db: Arc<DbBridge>,
    ) -> Self {
        Self {
            provider,
            llm_config,
            db,
        }
    }
}

/// 把 async future 在一条独立线程的独立运行时上跑成同步结果。
///
/// 刻意收成泛型函数而不是内联在 `start` 里：`start` 返回的 future 必须 `Send +
/// 'static`（trait 要求），所以这里只接**不借用外部**的 future，调用方自己把
/// 需要的东西 `clone` 进来。
fn run_blocking<F>(fut: F) -> Result<F::Output, AdapterError>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("quill-member-exec".to_string())
        .spawn(move || {
            let out: Result<F::Output, AdapterError> =
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => Ok(rt.block_on(fut)),
                    Err(e) => Err(AdapterError::Internal(format!(
                        "起不起成员执行器的运行环境：{e}。下一步：重启服务再试；\
                         若反复出现，检查这台机器的线程 / 内存是否已耗尽。"
                    ))),
                };
            let _ = tx.send(out);
        })
        .map_err(|e| {
            AdapterError::Internal(format!(
                "起不起成员执行线程：{e}。下一步：重启服务再试；\
                 若反复出现，检查这台机器的线程数是否已耗尽。"
            ))
        })?;
    rx.recv().unwrap_or_else(|_| {
        Err(AdapterError::Internal(
            "成员执行线程没有回传结果（线程在返回前退出了）".to_string(),
        ))
    })
}

/// 读专家人格（`experts.instructions`）。读不到就 `None` —— **不编一个**。
fn persona(
    db: &DbBridge,
    owner: UserId,
    expert: &ExpertId,
) -> Result<Option<String>, AdapterError> {
    let b = blob_of(&owner);
    let id = expert.as_str().to_string();
    let out: Result<Option<String>, quill_agent::AgentError> = db.call(move |pool, _rt| {
        Box::pin(async move {
            let row: Option<Option<String>> = sqlx::query_scalar(
                "SELECT instructions FROM experts \
                 WHERE owner_user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(b)
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| storage_error("读专家人格", e))?;
            Ok(row.flatten())
        })
    });
    out.map_err(|e| AdapterError::Storage(format!("读专家人格失败：{e}")))
}

/// 真正跑一轮：拼 system + user，调一次模型，回正文。
///
/// 拿走全部所需值（不借用 `ProviderMemberExecutor`），这样它能被扔进
/// `run_blocking` 的 `'static` future 里。
async fn run_member(
    provider: SharedProvider,
    llm_config: crate::llm::LlmConfig,
    db: Arc<DbBridge>,
    req: MemberStartRequest,
) -> Result<String, AdapterError> {
    let mut system = String::from(DELEGATION_PREAMBLE);
    if let Some(p) = persona(&db, req.owner(), req.expert())?
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        system.push_str("\n\n你的身份与风格：\n");
        system.push_str(p);
    }
    let user = format!("任务：{}\n\n{}", req.title(), req.instructions());
    let request = crate::llm::build_request(
        &llm_config,
        vec![LlmMessage::system(system), LlmMessage::user(user)],
    );
    let resp = provider.chat(&request).await.map_err(|e| {
        AdapterError::Provider(format!("成员 {} 的模型调用失败：{e}", req.member()))
    })?;
    Ok(resp.text)
}

impl MemberExecutor for ProviderMemberExecutor {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send {
        // **在返回 future 之前就把活干完**（见文件头：dispatch 用空 waker 自旋 poll）。
        let member = req.member().clone();
        let scope = req.title().to_string();
        let provider = Arc::clone(&self.provider);
        let llm_config = self.llm_config.clone();
        let db = Arc::clone(&self.db);
        // `run_blocking` 外层是「线程 / 运行时起没起来」，内层是任务本身的结果；
        // 对调用方这两层是同一件事（都算这次派工失败），所以拍平。
        let done = run_blocking(async move { run_member(provider, llm_config, db, req).await })
            .and_then(|inner| inner);

        async move {
            match done {
                // 模型回了正文 → 交付。
                Ok(text) if !text.trim().is_empty() => MemberOutcome::done(member, &scope, &text)
                    .map_err(|e| AdapterError::Internal(format!("构造成员产出失败：{e}"))),
                // 模型回了个空串 → 这个成员没产出，按失败报，**不编一段话**。
                Ok(_) => MemberOutcome::failed(member, &scope)
                    .map_err(|e| AdapterError::Internal(format!("构造成员失败产出失败：{e}"))),
                Err(e) => Err(e),
            }
        }
    }

    fn steer(
        &self,
        member: &MemberId,
        _m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        // 诚实：这一版**没有**运行中追加指令的通道。返回 `Ok(())` 会是谎话 ——
        // 调用方会以为指令送到了。等真有长跑成员时再做。
        let detail = format!("steer 尚未实现：成员 {member} 无法在运行中接收追加指令");
        async move { Err(AdapterError::Internal(detail)) }
    }

    fn abort(
        &self,
        member: &MemberId,
        _scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        let detail = format!("abort 尚未实现：成员 {member} 的任务无法中途取消");
        async move { Err(AdapterError::Internal(detail)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use quill_provider::{ChatRequest, ChatResponse, ModelInfo, ProviderError, ProviderStream};

    /// 记录收到的请求、回一段固定正文的桩 provider。
    #[derive(Debug)]
    struct Recorder {
        reply: String,
        seen: Mutex<Vec<(String, String)>>,
    }

    impl Recorder {
        fn new(reply: &str) -> Self {
            Self {
                reply: reply.to_string(),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    fn text_of(m: &LlmMessage) -> String {
        match &m.content {
            quill_provider::MessageContent::Text { text } => text.clone(),
            other => format!("{other:?}"),
        }
    }

    impl quill_provider::Provider for Recorder {
        fn name(&self) -> &str {
            "recorder"
        }

        fn chat<'a>(
            &'a self,
            request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ChatResponse> {
            let sys = request
                .messages
                .iter()
                .find(|m| matches!(m.role, quill_provider::Role::System))
                .map(text_of)
                .unwrap_or_default();
            let user = request
                .messages
                .iter()
                .find(|m| matches!(m.role, quill_provider::Role::User))
                .map(text_of)
                .unwrap_or_default();
            self.seen.lock().expect("测试锁不该毒化").push((sys, user));
            let reply = self.reply.clone();
            let model = request.model.clone();
            Box::pin(async move {
                Ok(ChatResponse {
                    id: None,
                    model,
                    text: reply,
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish_reason: None,
                    usage: Default::default(),
                })
            })
        }

        fn stream<'a>(
            &'a self,
            _r: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ProviderStream> {
            Box::pin(async {
                Err(ProviderError::NotConfigured {
                    detail: "单测不用流式".into(),
                })
            })
        }

        fn models<'a>(&'a self) -> quill_provider::BoxFuture<'a, Vec<ModelInfo>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    /// 空 waker 自旋 poll —— 与 `quill-agent` 的 `block_on` 逐字同构，
    /// 用来钉住「`start` 返回的 future 必须第一次 poll 就 Ready」这条契约。
    fn poll_noop<F: std::future::Future>(fut: F) -> Option<F::Output> {
        use std::task::{Context, Poll};
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = Box::pin(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => Some(v),
            Poll::Pending => None,
        }
    }

    /// 全部迁移拼成一条 SQL —— 与 `tests/common/mod.rs` 的 `migration_sql` 同源
    /// （顺序一律从 `quill_store::MIGRATIONS` 取，不手写文件名清单）。
    fn migration_sql() -> String {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../quill-store/migrations");
        let mut buf = String::new();
        for m in quill_store::MIGRATIONS {
            let path = dir.join(format!("{}.sql", m.name));
            buf.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("读取迁移文件 {} 失败：{e}", path.display())),
            );
            buf.push('\n');
        }
        buf
    }

    fn temp_db(label: &str) -> (Arc<DbBridge>, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "quill-member-exec-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("quill.db");
        let db = DbBridge::open(path.to_str().expect("临时路径是 UTF-8"), 2).expect("开临时库");
        let sql = migration_sql();
        let n = db.migrate(&sql).expect("跑迁移");
        assert!(n > 0, "迁移必须真的执行了语句");
        (Arc::new(db), dir)
    }

    fn seed_expert(db: &DbBridge, owner: UserId, id: &str, instructions: &str) {
        let owner_b = blob_of(&owner);
        let id = id.to_string();
        let instructions = instructions.to_string();
        db.call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO experts (id, owner_user_id, display_name, version, description, \
                     role_summary, visibility, tool_policy_json, tags_json, license, \
                     default_enabled, is_builtin, asset_hash, persona_hash, \
                     created_at, updated_at, instructions, model, source_template) \
                     VALUES (?, ?, '显示名', '0.1.0', '', '', 'manual_enable', '{}', '[]', '', \
                     1, 0, zeroblob(32), zeroblob(32), 1, 1, ?, NULL, NULL)",
                )
                .bind(id)
                .bind(owner_b)
                .bind(instructions)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("铺专家", e))?;
                Ok(())
            })
        })
        .expect("铺专家行");
    }

    /// 派工请求参数：只关心模型名，其余走默认。
    fn llm_cfg() -> crate::llm::LlmConfig {
        crate::llm::LlmConfig {
            model: "stub-model".to_string(),
            ..Default::default()
        }
    }

    fn req(owner: UserId, expert: &str, member: &str) -> MemberStartRequest {
        MemberStartRequest::new(
            owner,
            quill_adapters::SessionId::from_bytes([7; 16]),
            ExpertId::parse(expert).expect("专家名合法"),
            MemberId::parse(member).expect("成员标识合法"),
            "分析 Q3 成本",
            "请给出三点结论",
        )
        .expect("启动请求合法")
    }

    #[test]
    fn start_returns_a_future_that_is_ready_on_the_first_poll() {
        // 这条是**契约**：dispatch 用空 waker 自旋 poll，一次 Pending 就会空转到 panic。
        let (db, dir) = temp_db("ready");
        let owner = UserId::from_bytes([1; 16]);
        let exec = ProviderMemberExecutor::new(
            Arc::new(Recorder::new("三点结论……")),
            llm_cfg(),
            Arc::clone(&db),
        );
        let outcome = poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("start 必须在第一次 poll 就完成");
        let outcome = outcome.expect("桩 provider 必然成功");
        assert_eq!(outcome.status().as_wire(), "done");
        assert_eq!(outcome.completed_scope(), "分析 Q3 成本");
        assert_eq!(outcome.output(), "三点结论……");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn persona_is_injected_into_the_system_prompt() {
        let (db, dir) = temp_db("persona");
        let owner = UserId::from_bytes([2; 16]);
        seed_expert(&db, owner, "cost-analyst", "你是一个只谈钱的分析师。");
        let recorder = Arc::new(Recorder::new("好"));
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&recorder) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let out = poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("第一次 poll 就完成")
            .expect("应成功");
        assert_eq!(out.status().as_wire(), "done");
        let seen = recorder.seen.lock().expect("锁不毒化");
        let (sys, user) = &seen[0];
        assert!(
            sys.contains("被委派的团队成员"),
            "固定前缀必须进 system：{sys}"
        );
        assert!(
            sys.contains("只谈钱的分析师"),
            "专家人格必须进 system：{sys}"
        );
        assert!(user.contains("分析 Q3 成本"), "任务标题必须进 user：{user}");
        assert!(
            user.contains("请给出三点结论"),
            "任务正文必须进 user：{user}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_expert_row_still_runs_and_says_no_persona_was_found() {
        // 没有 experts 行时不该失败：人格是可选的，任务本身要能跑。
        let (db, dir) = temp_db("nopersona");
        let owner = UserId::from_bytes([3; 16]);
        let recorder = Arc::new(Recorder::new("勉强给个结论"));
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&recorder) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let out = poll_noop(exec.start(req(owner, "ghost-expert", "ghost-expert-1")))
            .expect("第一次 poll 就完成")
            .expect("缺人格不应失败");
        assert_eq!(out.status().as_wire(), "done");
        let seen = recorder.seen.lock().expect("锁不毒化");
        assert!(!seen[0].0.contains("身份与风格"), "没有人格时不该编一段");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_model_reply_is_a_failure_not_a_delivery() {
        // 空正文 = 成员没产出。按 Done 会造出一份空交付，汇总时会凭空少一块。
        let (db, dir) = temp_db("empty");
        let owner = UserId::from_bytes([4; 16]);
        let exec =
            ProviderMemberExecutor::new(Arc::new(Recorder::new("   ")), llm_cfg(), Arc::clone(&db));
        let out = poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("第一次 poll 就完成")
            .expect("空回复不是 Err，是「失败」这个状态");
        assert_eq!(out.status().as_wire(), "failed");
        assert!(out.output().trim().is_empty(), "失败产出必须为空");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn steer_and_abort_say_they_are_not_supported_instead_of_pretending() {
        let (db, _dir) = temp_db("steer");
        let exec =
            ProviderMemberExecutor::new(Arc::new(Recorder::new("x")), llm_cfg(), Arc::clone(&db));
        let member = MemberId::parse("cost-analyst-1").expect("成员标识合法");
        let msg = Message::user("停一下").expect("消息合法");
        let steered = poll_noop(exec.steer(&member, msg)).expect("第一次 poll 就完成");
        assert!(steered.is_err(), "尚未支持就必须报错，不能假装成功");
        let aborted =
            poll_noop(exec.abort(&member, AbortScope::StopRound)).expect("第一次 poll 就完成");
        assert!(aborted.is_err(), "尚未支持就必须报错，不能假装成功");
    }
}
