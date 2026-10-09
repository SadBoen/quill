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
//!
//! **运行中的成员是可控的（queue Q023/Q024）**：`start` 时把这一位登记进
//! [`crate::member_control::MemberControl`]，于是 `steer`（追加指令）与 `abort`
//! （中途取消）有可作用的对象。成员在**两轮模型调用之间**取走追加指令 ——
//! 与 goose 的 `SteerOperation` 同一个窗口（`crate::member_control` 的模块头
//! 列了出处）。没有追加指令时仍然只调一次模型，老行为一个字不变。

use std::sync::Arc;

use quill_adapters::{
    AbortScope, AdapterError, ExpertId, MemberExecutor, MemberId, MemberOutcome,
    MemberStartRequest, MemberStatus, MemberUsage, Message, UserId,
};
use quill_provider::{Message as LlmMessage, SharedProvider};

use crate::db::{blob_of, storage_error, DbBridge};
use crate::member_control::{MemberControl, MemberRun};

/// 子 agent 的固定前缀指令。
///
/// 三件事必须说清（都是子 agent 的结构决定的，不是文风）：
/// 它是**被委派的成员**、产出会被主持人汇总、**期间没有交互通道** ——
/// 子 agent 没有人可以回答问题，所以它不该提问，只能给出可用的结论。
const DELEGATION_PREAMBLE: &str = "\
你是一个被委派的团队成员，正在独立完成主持人交给你的一个子任务。\
你的产出会被汇总给主持人，期间没有人与你交互，所以不要提问、不要请求确认 —— \
直接给出可用的结论。信息不足时，用你已有的知识做出最合理的判断，并标出不确定之处。";

/// 成员追问的标记（queue Q105）：模型回一行以它开头的文本 = 一次澄清请求。
const ASK_PREFIX: &str = "[提问]";

/// 允许追问时追加到 system 末尾的那段。
///
/// **它显式覆盖上面那句「不要提问」**，而不是与之并列 —— 否则模型收到的是一对
/// 自相矛盾的指令。同时说清本仓的追问语义：**没有人会回答**，追问只是换一次
/// 自我审视，收到后仍要自行决断（用户 2026-10-09 拍板的口径）。
fn ask_addendum(budget: u32) -> String {
    format!(
        "\n\n【关于追问】以下规则**覆盖上面那句「不要提问」**：你最多可以提出 {budget} 次澄清请求。\
         确实缺少必要信息时，**只**回一行以「{ASK_PREFIX}」开头的追问，不要写别的。\
         但请注意：**没有人会回答你** —— 追问只是让你重新审视一次，收到追问后你仍须依据已有信息自行决断；\
         仍不确定之处请显式标注。超过 {budget} 次追问会被判为失败。"
    )
}

/// 追问耗尽后注入的「自行决断」指令（无人回答时的唯一出路）。
const SELF_DECIDE_NUDGE: &str = "（系统）没有人可以回答你的追问。请依据已有信息自行决断，\
     直接给出可用的结论；仍不确定之处请显式标注。";

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
    /// 运行中成员的控制面。默认每个执行器自己一份；`with_control` 可以换成
    /// 进程里共享的那一份 —— 这样**另一个请求**（HTTP 的 steer/abort）才找得到
    /// 正在跑的成员（queue Q113 接线）。
    control: Arc<MemberControl>,
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
            control: Arc::new(MemberControl::new()),
        }
    }

    /// 换成共享的控制面（进程里一份时，另一个请求才找得到运行中的成员）。
    pub fn with_control(mut self, control: Arc<MemberControl>) -> Self {
        self.control = control;
        self
    }

    /// 这份执行器用的控制面（接线时把它放进 `AppState`）。
    pub fn control(&self) -> &Arc<MemberControl> {
        &self.control
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

/// 成员一次运行的收尾：正常拿到正文（连同这一轮烧掉的 token），还是被取消。
enum MemberRunEnd {
    Text(String, MemberUsage),
    Cancelled,
}

/// 一位成员最多跑几轮模型调用。
///
/// 只在**收到追加指令**时才加轮 —— 没有 steer 时就是一次调用，老行为一个字不变。
/// 上限是防「有人不停 steer」把成员拖成无底洞：到顶**如实失败**，不静默丢指令。
const MEMBER_MAX_ROUNDS: u32 = 8;

/// 真正把一位成员跑完：拼 system + 首轮任务，调模型；**两轮之间**取走追加指令
/// （goose 的 `SteerOperation` 就作用在这个窗口，见 [`crate::member_control`]）；
/// 被取消时立刻收手。
///
/// 拿走全部所需值（不借用 `ProviderMemberExecutor`），这样它能被扔进
/// `run_blocking` 的 `'static` future 里。
async fn run_member(
    provider: SharedProvider,
    llm_config: crate::llm::LlmConfig,
    db: Arc<DbBridge>,
    req: MemberStartRequest,
    run: Arc<MemberRun>,
) -> Result<MemberRunEnd, AdapterError> {
    let member = req.member().clone();
    let mut system = String::from(DELEGATION_PREAMBLE);
    // 预算 > 0 才追加那段；= 0 时提示词一个字不变（老行为）。
    // 注意预算来自团队列 `max_ask_depth`，**默认团队是 3 不是 0**（见
    // `quill_agent::team_limits`）—— 会走到这里的默认团队都会带上追问段。
    let ask_budget = req.max_ask_depth();
    if ask_budget > 0 {
        system.push_str(&ask_addendum(ask_budget));
    }
    if let Some(p) = persona(&db, req.owner(), req.expert())?
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        system.push_str("\n\n你的身份与风格：\n");
        system.push_str(p);
    }
    let mut messages = vec![
        LlmMessage::system(system),
        LlmMessage::user(format!("任务：{}\n\n{}", req.title(), req.instructions())),
    ];

    let mut rounds = 0u32;
    // 已经用掉的追问次数（queue Q105）。闸门是**用掉之前**判的：预算 N 就只放行
    // N 次追问，第 N+1 次如实失败 —— 与 `max_dispatch`/`max_replan` 同一条纪律
    // 「超限就拒绝，不静默截断」。
    let mut asks_used = 0u32;
    // 这一轮**所有**模型调用的 token 都记在这里，而不是只留最后一次 ——
    // 与对话循环（`quill_core::turn::TurnUsage`）同一个累加器、同一条语义：
    // 只加真值、缓存两项不并进入参、单轮逐字不变。steer 会让成员有多轮，
    // 只留最后一轮会把「成员这一轮烧了多少」少报一大截。
    let mut turn_usage = quill_core::turn::TurnUsage::default();
    loop {
        if run.is_cancelled() {
            return Ok(MemberRunEnd::Cancelled);
        }
        rounds += 1;
        let request = crate::llm::build_request(&llm_config, messages.clone());
        // **取消要能打断正在飞的模型调用**：select 会把败者丢掉，于是那一次请求
        // 当场作废 —— 这是 abort 的真身，不是「等它答完再把结果扔掉」。
        let waiter = run.cancelled();
        tokio::pin!(waiter);
        let resp = tokio::select! {
            r = provider.chat(&request) => r.map_err(|e| {
                AdapterError::Provider(format!("成员 {member} 的模型调用失败：{e}"))
            })?,
            _ = &mut waiter => return Ok(MemberRunEnd::Cancelled),
        };
        turn_usage.push(resp.usage);

        let text = resp.text;
        // 追问（Q105）：模型只回一行 `[提问] …`。**没有人会回答**，所以这不是
        // 「等人回话」，而是「再跑一轮、逼它自行决断」。预算用完仍追问 = 如实失败，
        // 不静默把追问当结论交出去（那会让主持人收到一句没头没尾的问题）。
        if ask_budget > 0 && text.trim_start().starts_with(ASK_PREFIX) {
            asks_used += 1;
            if asks_used > ask_budget {
                return Err(AdapterError::Internal(format!(
                    "成员 {member} 的追问超过上限（max_ask_depth = {ask_budget}，已就第 \
                     {asks_used} 次提问），无法再有更多澄清。\
                     下一步：把任务说明里缺的信息补全后重派，或 PATCH /api/teams/{{id}} \
                     调大 max_ask_depth（上限 5）。"
                )));
            }
            if rounds >= MEMBER_MAX_ROUNDS {
                return Err(AdapterError::Internal(format!(
                    "成员 {member} 的轮次超过上限（{MEMBER_MAX_ROUNDS} 轮，已跑 {rounds} 次模型调用）。\
                     下一步：把任务说明一次写全再重派。"
                )));
            }
            messages.push(LlmMessage::assistant(text));
            messages.push(LlmMessage::user(SELF_DECIDE_NUDGE));
            continue;
        }

        let pending = run.drain_steers();
        if pending.is_empty() {
            return Ok(MemberRunEnd::Text(text, usage_of(turn_usage.finish())));
        }
        if rounds >= MEMBER_MAX_ROUNDS {
            return Err(AdapterError::Internal(format!(
                "成员 {member} 的追加指令轮次超过上限（{MEMBER_MAX_ROUNDS} 轮，已跑 {rounds} 次模型调用）。\
                 下一步：少发几次 steer，或把要补充的内容一次写全再重派。"
            )));
        }
        // 这一轮的正文留在对话里，追加指令作为下一条 user 消息 —— 与 goose 在
        // 轮间把排队的引导注入对话是同一件事（`SteerOperation`）。
        messages.push(LlmMessage::assistant(text));
        for m in pending {
            messages.push(LlmMessage::user(format!("[追加指令] {}", m.text())));
        }
    }
}

/// 内核累加器的结果 → 适配层字段。只做搬运，不改口径（口径在
/// `TurnUsage::finish` 的文档里）。
fn usage_of(u: quill_provider::TokenUsage) -> MemberUsage {
    MemberUsage {
        input: u.input,
        output: u.output,
        cache_read: u.cache_read,
        cache_write: u.cache_write,
    }
}

impl MemberExecutor for ProviderMemberExecutor {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send {
        // **在返回 future 之前就把活干完**（见文件头：dispatch 用空 waker 自旋 poll）。
        let member = req.member().clone();
        let scope = req.title().to_string();
        let owner = req.owner();
        let provider = Arc::clone(&self.provider);
        let llm_config = self.llm_config.clone();
        let db = Arc::clone(&self.db);
        // 登记进控制面：**运行中**才收得到 steer / abort。守卫随 future 一起活着，
        // 成员跑完（含 panic）时把这一条摘掉。
        let (run, guard) = self.control.register(owner, member.clone());
        // `run_blocking` 外层是「线程 / 运行时起没起来」，内层是任务本身的结果；
        // 对调用方这两层是同一件事（都算这次派工失败），所以拍平。
        let done = run_blocking(async move {
            let _guard = guard;
            run_member(provider, llm_config, db, req, run).await
        })
        .and_then(|inner| inner);

        async move {
            match done {
                // 模型回了正文 → 交付，**连同这一轮烧掉的 token**（Q026）。
                Ok(MemberRunEnd::Text(text, usage)) if !text.trim().is_empty() => {
                    MemberOutcome::done(member, &scope, &text)
                        .map(|o| o.with_usage(usage))
                        .map_err(|e| AdapterError::Internal(format!("构造成员产出失败：{e}")))
                }
                // 模型回了个空串 → 这个成员没产出，按失败报，**不编一段话**。
                Ok(MemberRunEnd::Text(_, _)) => MemberOutcome::failed(member, &scope)
                    .map_err(|e| AdapterError::Internal(format!("构造成员失败产出失败：{e}"))),
                // 被 abort：产出为空、状态 cancelled —— 台账那边按
                // `MemberRejectKind::Cancelled` 结算（见 `quill_agent::dispatch::run_one`）。
                Ok(MemberRunEnd::Cancelled) => {
                    MemberOutcome::new(member, MemberStatus::Cancelled, &scope, "")
                        .map_err(|e| AdapterError::Internal(format!("构造成员取消产出失败：{e}")))
                }
                Err(e) => Err(e),
            }
        }
    }

    fn steer(
        &self,
        owner: &UserId,
        member: &MemberId,
        m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        let control = Arc::clone(&self.control);
        let owner = *owner;
        let member = member.clone();
        async move { control.steer(&owner, &member, m) }
    }

    fn abort(
        &self,
        owner: &UserId,
        member: &MemberId,
        scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        let control = Arc::clone(&self.control);
        let owner = *owner;
        let member = member.clone();
        async move { control.abort(&owner, &member, scope) }
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

    // ------------------------------------------------------- 运行中控制（Q023/Q024）

    /// 会**卡住**的桩 provider：第一次调用先报到、等放行，之后每次调用立刻回。
    ///
    /// 「卡住」是这组用例的前提：只有成员真的停在一次模型调用里，
    /// steer / abort 才有「运行中」这个窗口可作用（老实现没有这个窗口）。
    #[derive(Debug)]
    struct Gated {
        replies: Mutex<Vec<String>>,
        seen: Mutex<Vec<String>>,
        started: std::sync::mpsc::Sender<usize>,
        gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }

    impl quill_provider::Provider for Gated {
        fn name(&self) -> &str {
            "gated"
        }

        fn chat<'a>(
            &'a self,
            request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ChatResponse> {
            let user = request
                .messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, quill_provider::Role::User))
                .map(text_of)
                .unwrap_or_default();
            let n = {
                let mut seen = self.seen.lock().expect("锁不毒化");
                seen.push(user);
                seen.len()
            };
            let _ = self.started.send(n);
            let gate = self.gate.lock().expect("锁不毒化").take();
            let reply = self
                .replies
                .lock()
                .expect("锁不毒化")
                .get(n - 1)
                .cloned()
                .unwrap_or_default();
            let model = request.model.clone();
            Box::pin(async move {
                // 只有第一轮等放行。被取消时这个 future 会被 select **整体丢掉**，
                // 于是那一次调用当场作废 —— 这正是 abort 要证明的事。
                if let Some(rx) = gate {
                    let _ = rx.await;
                }
                Ok(ChatResponse {
                    id: None,
                    model,
                    text: reply,
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish_reason: None,
                    // 每轮上报一个**与轮次绑定**的数：第 n 轮 input=100n / output=10n。
                    // 这样「跨轮累加」被钉住 —— 只留最后一轮会得到 200/20 而不是 300/30。
                    usage: quill_provider::TokenUsage::new(
                        Some(100 * n as u32),
                        Some(10 * n as u32),
                    ),
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

    #[test]
    fn a_missing_usage_stays_missing_and_a_reported_one_survives() {
        // Q026：模型端没上报时用量必须保持全空（不是 0）；上报了就原样带下来。
        let (db, dir) = temp_db("usage");
        let owner = UserId::from_bytes([8; 16]);

        // 桩 provider 不报 usage（`Recorder`）→ 用量全 None。
        let silent = ProviderMemberExecutor::new(
            Arc::new(Recorder::new("结论")) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let out = poll_noop(silent.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("第一次 poll 就完成")
            .expect("应成功");
        assert_eq!(
            out.usage(),
            MemberUsage::default(),
            "上游没上报时报 0 就是编数字"
        );

        // 桩 provider 报 usage → 单轮逐字带下来。
        let verbose = ProviderMemberExecutor::new(
            Arc::new(UsageReporter {
                usage: quill_provider::TokenUsage::new(Some(11), Some(7))
                    .with_cache(Some(3), Some(2)),
            }) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let out = poll_noop(verbose.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("第一次 poll 就完成")
            .expect("应成功");
        assert_eq!(
            out.usage(),
            MemberUsage {
                input: Some(11),
                output: Some(7),
                cache_read: Some(3),
                cache_write: Some(2),
            },
            "单轮上报的用量必须原样带下来"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 只上报 usage 的桩 provider（正文固定）。
    #[derive(Debug)]
    struct UsageReporter {
        usage: quill_provider::TokenUsage,
    }

    impl quill_provider::Provider for UsageReporter {
        fn name(&self) -> &str {
            "usage-reporter"
        }

        fn chat<'a>(
            &'a self,
            request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ChatResponse> {
            let usage = self.usage;
            let model = request.model.clone();
            Box::pin(async move {
                Ok(ChatResponse {
                    id: None,
                    model,
                    text: "结论".to_string(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish_reason: None,
                    usage,
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

    /// 在**真运行中**的成员身上追加一条指令：下一轮必须看得见它，交付的是新一轮。
    #[test]
    fn steer_reaches_a_running_member_and_the_member_answers_again() {
        let (db, dir) = temp_db("steer-live");
        let owner = UserId::from_bytes([5; 16]);
        let member = MemberId::parse("cost-analyst-1").expect("成员标识合法");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let provider = Arc::new(Gated {
            replies: Mutex::new(vec!["第一轮答案".into(), "第二轮答案".into()]),
            seen: Mutex::new(Vec::new()),
            started: started_tx,
            gate: Mutex::new(Some(release_rx)),
        });
        let control = Arc::new(MemberControl::new());
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&provider) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        )
        .with_control(Arc::clone(&control));

        let worker = std::thread::spawn(move || {
            poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
                .expect("第一次 poll 就完成")
                .expect("桩 provider 必然成功")
        });

        // 第一次调用报到 = 成员**正在跑**。
        assert_eq!(started_rx.recv().expect("第一次调用必须报到"), 1);
        control
            .steer(
                &owner,
                &member,
                Message::user("补充：请附上数据来源").expect("消息合法"),
            )
            .expect("运行中的成员必须收得下追加指令");
        release_tx.send(()).expect("放行第一轮");

        // 第二轮报到 = 追加指令真的换来了又一轮。用 `recv_timeout` 而不是 `recv`：
        // 指令被吞掉时（没有第二轮）这条要**变红**，不能把测试套件挂死。
        let second = started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("收到追加指令却不跑第二轮，等于把指令吞了");
        assert_eq!(second, 2);

        let outcome = worker.join().expect("线程不该 panic");
        assert_eq!(outcome.status().as_wire(), "done", "{outcome:?}");
        assert_eq!(outcome.output(), "第二轮答案", "要交付**最后一轮**的正文");

        // Q026：两轮的 token 必须**累加**（300/30），不是只留最后一轮（200/20）。
        assert_eq!(
            outcome.usage(),
            MemberUsage {
                input: Some(300),
                output: Some(30),
                cache_read: None,
                cache_write: None,
            },
            "成员这一轮的用量必须是**跨轮累计**，不是最后一次调用"
        );

        let seen = provider.seen.lock().expect("锁不毒化").clone();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(
            seen[1].contains("[追加指令] 补充：请附上数据来源"),
            "第二轮的提示词里必须带着追加指令：{seen:?}"
        );
        assert!(control.running(&owner).is_empty(), "跑完必须摘掉注册");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// abort：正在飞的模型调用当场作废，成员按 `cancelled` 结算，且**没有第二轮**。
    #[test]
    fn abort_cancels_a_running_member_mid_call() {
        let (db, dir) = temp_db("abort-live");
        let owner = UserId::from_bytes([6; 16]);
        let member = MemberId::parse("cost-analyst-1").expect("成员标识合法");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let provider = Arc::new(Gated {
            replies: Mutex::new(vec!["这一轮不该被交付".into()]),
            seen: Mutex::new(Vec::new()),
            started: started_tx,
            gate: Mutex::new(Some(release_rx)),
        });
        let control = Arc::new(MemberControl::new());
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&provider) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        )
        .with_control(Arc::clone(&control));

        let worker = std::thread::spawn(move || {
            poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
                .expect("第一次 poll 就完成")
                .expect("取消是 cancelled 这个状态，不是 Err")
        });

        assert_eq!(started_rx.recv().expect("第一次调用必须报到"), 1);
        // **兜底放行**：取消万一没实现，这条用例要变红而不是把测试套件挂死。
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = release_tx.send(());
        });
        control
            .abort(&owner, &member, AbortScope::StopRound)
            .expect("运行中的成员必须取消得掉");

        let outcome = worker.join().expect("线程不该 panic");
        assert_eq!(outcome.status().as_wire(), "cancelled", "{outcome:?}");
        assert!(outcome.output().trim().is_empty(), "取消没有产出");
        assert_eq!(
            provider.seen.lock().expect("锁不毒化").len(),
            1,
            "取消之后不许再跑一轮"
        );
        assert!(control.running(&owner).is_empty(), "跑完必须摘掉注册");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 没有在跑的成员：steer / abort **如实报错** —— 不假装成功、也不排队等下次。
    #[test]
    fn steer_and_abort_on_an_idle_member_are_honest_errors() {
        let (db, _dir) = temp_db("steer-idle");
        let owner = UserId::from_bytes([7; 16]);
        let exec =
            ProviderMemberExecutor::new(Arc::new(Recorder::new("x")), llm_cfg(), Arc::clone(&db));
        let member = MemberId::parse("cost-analyst-1").expect("成员标识合法");
        let msg = Message::user("停一下").expect("消息合法");
        let err = poll_noop(exec.steer(&owner, &member, msg))
            .expect("第一次 poll 就完成")
            .expect_err("没有在跑的成员必须报错");
        assert!(format!("{err}").contains("没有正在运行的成员"), "{err}");
        assert!(format!("{err}").contains("下一步"), "{err}");
        let err = poll_noop(exec.abort(&owner, &member, AbortScope::StopRound))
            .expect("第一次 poll 就完成")
            .expect_err("没有在跑的成员必须报错");
        assert!(format!("{err}").contains("没有正在运行的成员"), "{err}");
    }

    // ------------------------------------------------------ 追问闸门（Q105）

    /// 按脚本逐轮回话的桩 provider（第 n 次调用回第 n 条，用光后回最后一条）。
    #[derive(Debug)]
    struct Scripted {
        replies: Mutex<Vec<String>>,
        calls: std::sync::atomic::AtomicUsize,
        seen_user: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(replies: &[&str]) -> Self {
            Self {
                replies: Mutex::new(replies.iter().map(|s| s.to_string()).collect()),
                calls: std::sync::atomic::AtomicUsize::new(0),
                seen_user: Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl quill_provider::Provider for Scripted {
        fn name(&self) -> &str {
            "scripted"
        }

        fn chat<'a>(
            &'a self,
            request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ChatResponse> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.seen_user
                .lock()
                .expect("锁不毒化")
                .extend(request.messages.iter().rev().take(1).map(text_of));
            let replies = self.replies.lock().expect("锁不毒化");
            let reply = replies
                .get(n)
                .or_else(|| replies.last())
                .cloned()
                .unwrap_or_default();
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

    /// `max_ask_depth = 0`（默认）：提示词**逐字**保持老样子，成员不会追问 —— 回一句
    /// `[提问]` 也被当成普通正文交付，不问不拦（老行为一个字不变）。
    #[test]
    fn ask_depth_zero_keeps_the_old_behavior_and_the_old_prompt() {
        let (db, dir) = temp_db("ask-off");
        let owner = UserId::from_bytes([9; 16]);
        let provider = Arc::new(Scripted::new(&["[提问] 这算不算问题？"]));
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&provider) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        // 不调 with_max_ask_depth → 默认 0。
        let out = poll_noop(exec.start(req(owner, "cost-analyst", "cost-analyst-1")))
            .expect("第一次 poll 就完成")
            .expect("应成功");
        assert_eq!(out.status().as_wire(), "done");
        assert_eq!(
            out.output(),
            "[提问] 这算不算问题？",
            "不许追问时，[提问] 只是普通正文，原样交付"
        );
        assert_eq!(provider.call_count(), 1, "不许追问就只调一次模型");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 允许追问：模型第一次回 `[提问]`，执行器注入「自行决断」再跑一轮；交付的是
    /// **第二轮**的正文（不是那句追问）。
    #[test]
    fn an_ask_is_answered_with_a_self_decide_nudge_and_the_member_runs_again() {
        let (db, dir) = temp_db("ask-on");
        let owner = UserId::from_bytes([10; 16]);
        let provider = Arc::new(Scripted::new(&[
            "[提问] 该用哪种口径？",
            "结论：按成本口径，降 12%。",
        ]));
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&provider) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let r = req(owner, "cost-analyst", "cost-analyst-1").with_max_ask_depth(2);
        let out = poll_noop(exec.start(r))
            .expect("第一次 poll 就完成")
            .expect("应成功");
        assert_eq!(out.status().as_wire(), "done");
        assert_eq!(
            out.output(),
            "结论：按成本口径，降 12%。",
            "交付的必须是自行决断之后的正文，不是那句追问"
        );
        assert_eq!(provider.call_count(), 2, "追问要换来第二轮模型调用");
        let seen = provider.seen_user.lock().expect("锁不毒化").clone();
        assert!(
            seen.iter().any(|t| t.contains("没有人可以回答你的追问")),
            "第二轮必须带上「自行决断」的注入：{seen:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 追问用满还继续追问 → **如实失败**（不把追问当结论交出去），且失败原因点名
    /// `max_ask_depth` 与上限。
    #[test]
    fn asking_beyond_the_budget_is_an_honest_failure() {
        let (db, dir) = temp_db("ask-over");
        let owner = UserId::from_bytes([11; 16]);
        // 每次都追问 → 第 2 次就超过 budget=1。
        let provider = Arc::new(Scripted::new(&["[提问] 还是不确定"]));
        let exec = ProviderMemberExecutor::new(
            Arc::clone(&provider) as SharedProvider,
            llm_cfg(),
            Arc::clone(&db),
        );
        let r = req(owner, "cost-analyst", "cost-analyst-1").with_max_ask_depth(1);
        let err = poll_noop(exec.start(r))
            .expect("第一次 poll 就完成")
            .expect_err("用满还追问必须如实报错");
        let msg = format!("{err}");
        assert!(msg.contains("追问超过上限"), "{msg}");
        assert!(msg.contains("max_ask_depth"), "{msg}");
        assert!(msg.contains("下一步"), "{msg}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
