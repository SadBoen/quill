//! 上游失败的重试与退避（queue Q022）。
//!
//! **判据**：上游 5xx / 超时有退避重试，且可测。
//!
//! **现状**：quill 对上游 5xx / 超时**没有**任何退避重试 —— 一次失败就把错误
//! 原样抛给用户。本模块是它的唯一归属。
//!
//! # 参考源（file:line 均相对仓库根；对照 `vendor/goose` v1.53.0）
//!
//! 骨架头注点名的是 `vendor/goose/crates/goose/src/agents/retry.rs`。读完它和
//! 它的接线（`state_machine/ops_retry.rs`）后必须更正一处：那是 **agent 层的
//! 整轮重试** —— 一轮跑完、success checks 没过，就把会话回滚到 kickoff 再跑
//! 一轮，直到 `attempts >= max_retries` 才放弃并给用户一条
//! `Maximum retry attempts (...) exceeded`（`ops_retry.rs:250-283`）。它里面
//! **没有** 5xx/429/超时的分类，也**没有**退避公式。
//!
//! 真正的上游重试与退避在 `vendor/goose/crates/goose-provider-types/src/retry.rs`：
//! `RetryConfig` / `should_retry` / `delay_for_attempt` / `retry_operation`（正在
//! 发请求的那条路径上，`with_retry` 把每次 provider 调用包起来重跑）。Q022 的
//! 判据说的是它，所以**本模块抄的是它**；agent 层那份只用来核对「什么时候放弃」
//! 的措辞。分类所需的 HTTP status → 错误映射抄自
//! `vendor/goose/crates/goose-providers/src/http_status.rs`。
//!
//! ## 对照表（每一项都能回 `vendor/goose` 核对）
//!
//! | goose（file:line） | 抄了什么 | quill 落点 |
//! |---|---|---|
//! | `goose-provider-types/src/retry.rs:14-17` | 默认值：重试 3 次、初始 1s、×2、单次上限 30s | `DEFAULT_MAX_RETRIES` / `DEFAULT_INITIAL_RETRY_INTERVAL_MS` / `DEFAULT_BACKOFF_MULTIPLIER` / `DEFAULT_MAX_RETRY_INTERVAL_MS` |
//! | `goose-provider-types/src/retry.rs:19-32` | `RetryConfig` 五个字段（含 `transient_only`） | `RetryPolicy` |
//! | `goose-provider-types/src/retry.rs:34-44` | `Default`（`transient_only: false`，见 `:41`） | `RetryPolicy::default` |
//! | `goose-provider-types/src/retry.rs:46-65` | `new(...)` 与 `.transient_only()` 构造器 | 同名方法 |
//! | `goose-provider-types/src/retry.rs:71-87` | 退避公式：第 0 次为 0；`initial × multiplier^(attempt-1)`；与上限取 min（`:80`）；再乘抖动 `0.8 + rand×0.4`（`:82-84`） | `RetryPolicy::delay_for_attempt` |
//! | `goose-provider-types/src/retry.rs:90-105` | 永久性 4xx 的三个 marker 字符串（`:94-98`）+ Anthropic thinking 签名错误（`:100-105`） | `PERMANENT_REQUEST_FAILURE_MARKERS` / `is_permanent_request_failure` |
//! | `goose-provider-types/src/retry.rs:107-116` | `should_retry`：429 / 5xx / 网络错误可重试；永久 4xx 不重试；其余 4xx 由 `transient_only` 决定；别的错误一律不重试 | `should_retry` |
//! | `goose-provider-types/src/retry.rs:118-157` | `retry_operation`：attempts 计数，`should_retry && attempts < max_retries` 才退避重跑；429 带服务端 `retry_delay` 时用服务端值（`:142-148`），否则用 `delay_for_attempt`；到最后把**最后一次错误**原样返回 | `retry_operation` |
//! | `goose-provider-types/src/retry.rs:196-269` | `ProviderRetry::with_retry_config`：auth 单独刷新一次（`:207-232`）、`GOOSE_PROVIDER_SKIP_BACKOFF` 开关（`:251-261`） | **未移植**（差异 5、6） |
//! | `goose-providers/src/http_status.rs:41` | `MAX_RETRY_AFTER_SECS = 3600`：429 提示的硬上限 | `MAX_RETRY_AFTER_SECS` |
//! | `goose-providers/src/http_status.rs:47-63` | `extract_retry_after`：body 的 `error.metadata.retry_after_seconds` 优先，header 兜底 | `extract_retry_after`（header 只认整数秒，见差异 8） |
//! | `goose-providers/src/http_status.rs:68-74` | 只有有限、非负、且 ≤3600 的值才算数（防 `Duration::from_secs_f64` panic） | `retry_after_from_seconds` |
//! | `goose-providers/src/http_status.rs:83-92,98-112` | `Retry-After` 的整数秒与 HTTP-date 两种形式 | 只移植整数秒（差异 8） |
//! | `goose-providers/src/http_status.rs:244-312` | HTTP status → 错误分类：401/403 Auth、402 Credits、404/400/其它 4xx RequestFailed、413 ContextLength、429 RateLimit、其余 5xx ServerError | `from_http_status` |
//! | `goose-providers/src/http_status.rs:321-335,362-373` | 发送超时与读 body 超时都归 `NetworkError` | `UpstreamFailure::NetworkError`（判定在 wire 层，本模块只消费） |
//! | `goose-provider-types/src/errors.rs:8-53` | `ProviderError` 的 15 个变体 | 收窄为 7 个（差异 2） |
//! | `goose-provider-types/src/errors.rs:112-114` | `is_network_error`：连接失败 / 超时 / 无状态码的请求错误 → `NetworkError` | 同上，wire 层负责归类 |
//! | `goose-provider-types/src/formats/anthropic.rs:900-906` | `is_thinking_signature_error` | `is_thinking_signature_error` |
//! | `goose/src/agents/reply_parts.rs:416` | 流式路径把策略强制成 `.transient_only()` | 接线要求（差异 1） |
//! | `goose/src/agents/reply_parts.rs:417-465` | 重试时机：只在**流第一个 item 之前**失败时重试，且重试是重建整个 stream | `retry_operation` 的粒度：一次 operation = 一次完整上游调用，调用方决定包多大 |
//! | `goose/src/agents/state_machine/ops_retry.rs:250-283` | agent 层放弃条件与消息（`Maximum retry attempts (...) exceeded`） | 只对应「超过上限后放弃」；success checks / on_failure 未移植（差异 10） |
//!
//! # 与 goose 的差异（不假装逐行等同）
//!
//! 1. **命名与默认值**：goose 的 `RetryConfig` 在这里叫 `RetryPolicy`（避免与
//!    将来 agent 层 recipe 的 retry 配置重名）。字段与默认值一个不改：`Default`
//!    仍是 `transient_only: false`（`retry.rs:41`）——**接线时必须用
//!    `.transient_only()`**，否则 400/404 这类 `RequestFailed` 也会被重试；goose
//!    的流式调用路径正是这么做的（`reply_parts.rs:416`）。
//! 2. **错误类型收窄**：goose 的 `ProviderError` 有 15 个变体（带 reqwest 状态码、
//!    URL 等），本模块只需要分类所需的最小集，收窄为 7 个变体
//!    [`UpstreamFailure`]，不依赖 reqwest / chrono / provider 实现。
//! 3. **`should_retry` 的兜底改成穷举**：goose 用 `_ => false`（`retry.rs:114`），
//!    quill 把每个不可重试变体显式列出 —— 新增变体时编译器强制表态。
//! 4. **抖动来源注入**：goose 直接调 `rand::random::<f64>()`（`retry.rs:82`）；
//!    quill-core 没有 rand 依赖，且测试要确定性，所以抖动单位区间值由调用方
//!    注入（`jitter_unit: f64`，契约 `[0,1)`）。防御性 clamp 到 `[0,1]`，因此
//!    quill 的因子区间是闭的 `[0.8, 1.2]`，goose 是 `[0.8, 1.2)`。
//! 5. **sleep 注入**：goose 硬编码 `tokio::time::sleep`（`retry.rs:12,150`），还为
//!    wasm 备了 `gloo_timers`（`retry.rs:8-12`）。quill 把 sleep 作为参数注入：
//!    单测零真实等待（否则本模块的退避测试要真睡几十秒），模块也不必绑定某个
//!    运行时的时钟。接线时传 `tokio::time::sleep` 即可。
//! 6. **未移植 `GOOSE_PROVIDER_SKIP_BACKOFF`**（`retry.rs:251-261`）：开关的注入点
//!    就是第 5 条的 sleep 参数 —— 接线层想跳过退避，传一个 no-op sleep 就行。
//! 7. **未移植 auth 单次刷新**（`retry.rs:207-232`）：那需要 provider 的
//!    `refresh_credentials()`（凭据状态），不属于纯重试逻辑；需要时在接线层加。
//! 8. **未移植 HTTP-date 形式的 `Retry-After`**（`http_status.rs:83-92,98-112`）：
//!    quill-core 没有日期库，本轮也不改 `Cargo.toml`。只移植数值形式（body 秒 /
//!    header 整数秒），3600s 上限逐字一致；wire 层若拿到 HTTP-date，自行换算成
//!    秒后传入。
//! 9. **未移植 400 的 context-length 嗅探**（`http_status.rs:114-242`）：
//!    `from_http_status` 把 400 一律判 `RequestFailed`；wire 层若已确认是上下文
//!    超限，直接构造 `ContextLengthExceeded`。413 与 goose 一致直接归入。
//! 10. **未移植 agent 层整轮重试**（`goose/src/agents/retry.rs`：success checks /
//!     `on_failure` 命令 / 会话回滚）：Q022 是传输层退避；那套「跑完一轮再判断」
//!     属于会话状态机的事（quill 侧见 `state_machine.rs`），若要做另立条目。
//!
//! # 未接线（截至 2026-10-08，如实记录）
//!
//! 本模块目前**零调用方**：重试**还没有**接到 `quill-server` 的 provider 调用
//! 路径上（那是 Q012 的端口接线）。当时 `grep` 的原文输出：
//!
//! ```text
//! $ grep -rn "quill_core::retry\|retry::" crates/ --include=*.rs | grep -v crates/quill-core/src/retry.rs
//! （无输出）
//! $ grep -rln "RetryPolicy\|retry_operation\|should_retry\|delay_for_attempt\|UpstreamFailure" \
//!       crates/quill-server/src crates/quill-provider/src crates/quill-agent/src
//! （无输出）
//! ```
//!
//! 也就是说：`quill-provider/src/error.rs:42` 的 `is_retryable()` 只把「可重试」
//! 报给用户，`quill-server` 的 `api_dispatch.rs:421` / `dispatch_ledger.rs:243`
//! 只是把它写进错误信封的 `retryable` 字段 —— **没有任何地方会自动重跑**。
//! 接线（Q012）时要做的三件事：
//!
//! 1. 把 `quill_provider::ProviderError` 翻译成 [`UpstreamFailure`]：
//!    `Timeout` / `Unreachable` → `NetworkError`；`Status { code, .. }` →
//!    `from_http_status(code, message)`；429 的 `Retry-After` 经
//!    [`extract_retry_after`] 填进 `retry_delay`。
//! 2. 用 `RetryPolicy::default().transient_only()` 作策略（与 goose 流式路径一致）。
//! 3. sleep 注入 `tokio::time::sleep`，抖动注入随机源（`rand` 或等价物）。

use std::future::Future;
use std::time::Duration;

/// 默认重试次数。移植自 `goose-provider-types/src/retry.rs:14`。
pub const DEFAULT_MAX_RETRIES: usize = 3;

/// 默认初始退避间隔（毫秒）。移植自 `goose-provider-types/src/retry.rs:15`。
pub const DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 1_000;

/// 默认退避倍数（指数退避）。移植自 `goose-provider-types/src/retry.rs:16`。
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;

/// 默认单次退避上限（毫秒）。移植自 `goose-provider-types/src/retry.rs:17`。
pub const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 30_000;

/// 抖动因子下界。移植自 `goose-provider-types/src/retry.rs:82`（`0.8 + rand × 0.4`）。
const JITTER_LOW_FACTOR: f64 = 0.8;

/// 抖动因子的跨度。移植自 `goose-provider-types/src/retry.rs:82`。
const JITTER_SPAN: f64 = 0.4;

/// 429 提示的硬上限（秒）：`1e30` 之类的病态值必须退化成「没有提示」，
/// 而不是把 agent 冻住或让 `Duration::from_secs_f64` panic。
/// 移植自 `goose-providers/src/http_status.rs:36-41`。
pub const MAX_RETRY_AFTER_SECS: f64 = 3600.0;

/// 一次上游失败，按「值不值得重试」分类。
///
/// 这是 goose `ProviderError`（`goose-provider-types/src/errors.rs:8-53`）里
/// 与重试判定相关的最小等价集：带 payload/URL 的展示信息留在 wire 层，
/// 本模块只关心类别与 429 的服务端提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamFailure {
    /// 429。移植自 `ProviderError::RateLimitExceeded`（`errors.rs:19-22`）。
    /// `retry_delay` 来自 `Retry-After` / body 提示；有值时优先于指数退避
    /// （`goose-provider-types/src/retry.rs:142-148`）。
    RateLimitExceeded {
        details: String,
        retry_delay: Option<Duration>,
    },
    /// 5xx。移植自 `ProviderError::ServerError`（`errors.rs:25`）。
    ServerError(String),
    /// 连接失败 / 请求超时 / 读 body 超时。
    /// 移植自 `ProviderError::NetworkError`（`errors.rs:28`）；归类依据见
    /// `errors.rs:112-114` 与 `http_status.rs:321-335,362-373`。
    NetworkError(String),
    /// 其余 4xx（400/404/408/…）。移植自 `ProviderError::RequestFailed`（`errors.rs:31`）。
    RequestFailed(String),
    /// 401/403。移植自 `ProviderError::Authentication`（`errors.rs:13`）。
    Authentication(String),
    /// 上下文超限。移植自 `ProviderError::ContextLengthExceeded`（`errors.rs:16`）。
    ContextLengthExceeded(String),
    /// 402。移植自 `ProviderError::CreditsExhausted`（`errors.rs:51-54`）。
    CreditsExhausted { details: String },
}

/// 重试策略。逐字段移植自 goose 的 `RetryConfig`
/// （`goose-provider-types/src/retry.rs:19-32`）。
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// 最多重试几次（不含第一次调用）。goose `max_retries`。
    pub max_retries: usize,
    /// 第一次退避的基数（毫秒）。goose `initial_interval_ms`。
    pub initial_interval_ms: u64,
    /// 指数退避的倍数。goose `backoff_multiplier`。
    pub backoff_multiplier: f64,
    /// 单次退避的上限（毫秒）。goose `max_interval_ms`。
    pub max_interval_ms: u64,
    /// 为 true 时只重试暂时性错误（5xx / 网络 / 429），4xx 一律不重试。
    /// goose `transient_only`（`retry.rs:29-31`）。
    pub transient_only: bool,
}

impl Default for RetryPolicy {
    /// 与 goose 的 `Default` 一致：3 次、1s、×2、30s、`transient_only: false`
    /// （`goose-provider-types/src/retry.rs:34-44`）。
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_interval_ms: DEFAULT_INITIAL_RETRY_INTERVAL_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            max_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            transient_only: false,
        }
    }
}

impl RetryPolicy {
    /// 移植自 `goose-provider-types/src/retry.rs:47-60`。
    pub fn new(
        max_retries: usize,
        initial_interval_ms: u64,
        backoff_multiplier: f64,
        max_interval_ms: u64,
    ) -> Self {
        Self {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
            transient_only: false,
        }
    }

    /// 移植自 `goose-provider-types/src/retry.rs:62-65`。
    pub fn transient_only(mut self) -> Self {
        self.transient_only = true;
        self
    }

    /// 第 `attempt` 次重试前该退避多久（`attempt` 从 1 开始；0 表示不重试）。
    ///
    /// 公式逐行移植自 `goose-provider-types/src/retry.rs:71-87`：
    ///
    /// - `attempt == 0` → 0（`:72-74`）;
    /// - 基数 `initial × multiplier^(attempt-1)`（`:76-78`）；
    /// - 与 `max_interval_ms` 取 min（`:80`）—— 上限截断的是**基数**；
    /// - 再乘抖动因子 `0.8 + jitter_unit × 0.4`（`:82-84`），所以抖动后的
    ///   单次值可能到 `1.2 × max_interval_ms`（goose 的既定行为）。
    ///
    /// `jitter_unit` 是 `[0, 1)` 的随机单位值：goose 用 `rand::random::<f64>()`，
    /// quill 由调用方注入（见模块头差异 4）。越界值被 clamp 到 `[0, 1]`。
    pub fn delay_for_attempt(&self, attempt: usize, jitter_unit: f64) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let exponent = (attempt - 1) as u32;
        let base_delay_ms = (self.initial_interval_ms as f64
            * self.backoff_multiplier.powi(exponent as i32)) as u64;

        let capped_delay_ms = std::cmp::min(base_delay_ms, self.max_interval_ms);

        let jitter_factor = JITTER_LOW_FACTOR + (jitter_unit.clamp(0.0, 1.0) * JITTER_SPAN);
        Duration::from_millis((capped_delay_ms as f64 * jitter_factor) as u64)
    }
}

/// 判定一个 4xx `RequestFailed` 是否是**确定性永久失败**：同一份 payload 重试
/// 多少次都一样，重试只会浪费退避时间。
///
/// 三个 marker 逐字移植自 `goose-provider-types/src/retry.rs:94-98`。
const PERMANENT_REQUEST_FAILURE_MARKERS: &[&str] = &[
    "blocks in the latest assistant message cannot be modified",
    "must remain as they were in the original response",
    "Reasoning is mandatory for this endpoint",
];

/// 移植自 `goose-provider-types/src/retry.rs:100-105`。
fn is_permanent_request_failure(message: &str) -> bool {
    PERMANENT_REQUEST_FAILURE_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
        || is_thinking_signature_error(message)
}

/// 移植自 `goose-provider-types/src/formats/anthropic.rs:900-906`。
fn is_thinking_signature_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("thinking")
        && (lower.contains("signature")
            || lower.contains("cannot be modified")
            || lower.contains("block_binding"))
}

/// 这个失败值不值得重试。移植自 `goose-provider-types/src/retry.rs:107-116`。
///
/// | 失败 | 是否重试 |
/// |---|---|
/// | 429 `RateLimitExceeded` | 是 |
/// | 5xx `ServerError` | 是 |
/// | 网络错误 / 超时 `NetworkError` | 是 |
/// | 永久性 4xx（thinking 签名类，`retry.rs:112`） | 否（即使 `transient_only == false`） |
/// | 其余 4xx `RequestFailed` | 仅当 `transient_only == false`（goose 默认） |
/// | 401/403、上下文超限、402、其它 | 否 |
///
/// 差异 3：goose 的 `_ => false` 在这里改成穷举，新增变体会编译报错。
pub fn should_retry(error: &UpstreamFailure, policy: &RetryPolicy) -> bool {
    match error {
        UpstreamFailure::RateLimitExceeded { .. }
        | UpstreamFailure::ServerError(_)
        | UpstreamFailure::NetworkError(_) => true,
        UpstreamFailure::RequestFailed(message) if is_permanent_request_failure(message) => false,
        UpstreamFailure::RequestFailed(_) => !policy.transient_only,
        UpstreamFailure::Authentication(_)
        | UpstreamFailure::ContextLengthExceeded(_)
        | UpstreamFailure::CreditsExhausted { .. } => false,
    }
}

/// 把一次上游操作重跑到成功 / 放弃。
///
/// 逐行移植自 goose 的 `retry_operation`（`goose-provider-types/src/retry.rs:118-157`），
/// 外加两处注入（模块头差异 4、5）：
///
/// - `operation`：真正发请求的闭包（`Fn() -> Future`，与 goose 的签名一致）；
/// - `sleep`：退避怎么等（接线传 `tokio::time::sleep`，单测传记录器 + 立即就绪）；
/// - `jitter_unit`：每次退避取一个 `[0, 1)` 的随机单位值。
///
/// 语义：成功立刻返回；失败且 `should_retry && attempts < max_retries` 时计数 +1、
/// 退避（429 带 `retry_delay` 时用服务端值）后重跑；否则返回**最后一次**错误。
///
/// 为什么用闭包而不是 trait（模块内的取舍说明）：goose 自己就是闭包
/// （`F: Fn() -> Fut`），照抄签名最省核对；而且调用方不用为既有类型实现新 trait，
/// 单测可以直接注入「第 N 次返回什么」的假上游，不依赖 mock server。
pub async fn retry_operation<F, Fut, T, S, SF, J>(
    policy: &RetryPolicy,
    operation: F,
    mut sleep: S,
    mut jitter_unit: J,
) -> Result<T, UpstreamFailure>
where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<T, UpstreamFailure>> + Send,
    T: Send,
    S: FnMut(Duration) -> SF + Send,
    SF: Future<Output = ()> + Send,
    J: FnMut() -> f64 + Send,
{
    let mut attempts = 0;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if should_retry(&error, policy) && attempts < policy.max_retries {
                    attempts += 1;

                    let delay = match &error {
                        UpstreamFailure::RateLimitExceeded {
                            retry_delay: Some(d),
                            ..
                        } => *d,
                        _ => policy.delay_for_attempt(attempts, jitter_unit()),
                    };

                    sleep(delay).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// 429 响应里的秒数提示 → `Duration`。
///
/// 移植自 `goose-providers/src/http_status.rs:68-74`：`NaN`、负数、无穷一律
/// 当「没有提示」（`Duration::from_secs_f64` 对它们会 panic），超过
/// [`MAX_RETRY_AFTER_SECS`] 的部分截到上限。
pub fn retry_after_from_seconds(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(seconds.min(MAX_RETRY_AFTER_SECS)))
}

/// `Retry-After` header 的整数秒形式。
///
/// 移植自 `goose-providers/src/http_status.rs:84-85`；goose 还接受 HTTP-date
/// （`:87-91`、`:98-112`），未移植（模块头差异 8）。非法值返回 `None`，
/// 与 goose 的 `parse::<u64>()` 失败即 `None` 一致。
pub fn retry_after_from_header_seconds(value: &str) -> Option<Duration> {
    let seconds = value.trim().parse::<u64>().ok()?;
    retry_after_from_seconds(seconds as f64)
}

/// 从 429 的响应里取重试提示：body 的 `error.metadata.retry_after_seconds`
/// 优先（OpenRouter 形状，比 header 的整数秒更精确），header 兜底。
///
/// 移植自 `goose-providers/src/http_status.rs:47-63`，签名把 goose 的
/// `HeaderMap` 换成 `Option<&str>`（本模块不依赖 HTTP 类型）。
pub fn extract_retry_after(
    payload: Option<&serde_json::Value>,
    header: Option<&str>,
) -> Option<Duration> {
    if let Some(seconds) = payload
        .and_then(|p| p.get("error"))
        .and_then(|e| e.get("metadata"))
        .and_then(|m| m.get("retry_after_seconds"))
        .and_then(|v| v.as_f64())
    {
        if let Some(delay) = retry_after_from_seconds(seconds) {
            return Some(delay);
        }
    }

    header.and_then(retry_after_from_header_seconds)
}

/// HTTP 状态码 → [`UpstreamFailure`]，逐条移植自 goose 的
/// `map_http_error_to_provider_error`（`goose-providers/src/http_status.rs:244-312`）：
/// 401/403 → `Authentication`；402 → `CreditsExhausted`；413 → `ContextLengthExceeded`；
/// 400/404/其余 4xx → `RequestFailed`；429 → `RateLimitExceeded`；其余 5xx →
/// `ServerError`（`:290-294`）。`retry_after` 由调用方用 [`extract_retry_after`]
/// 补上（goose 在 `handle_status_with_limit` 里补，`:414-420`）。
///
/// 差异 9：goose 会把 400/413 的 body 再嗅探一遍，确认是上下文超限就改判
/// `ContextLengthExceeded`（`:114-242`、`:277-285`）；本函数不做 payload 嗅探，
/// 需要时由 wire 层直接构造该变体。
pub fn from_http_status(status: u16, message: &str) -> UpstreamFailure {
    match status {
        401 | 403 => UpstreamFailure::Authentication(format!(
            "Authentication failed ({}): {}",
            status, message
        )),
        404 => UpstreamFailure::RequestFailed(format!("Resource not found (404): {}", message)),
        402 => UpstreamFailure::CreditsExhausted {
            details: message.to_string(),
        },
        413 => UpstreamFailure::ContextLengthExceeded(message.to_string()),
        400 => UpstreamFailure::RequestFailed(format!("Bad request (400): {}", message)),
        429 => UpstreamFailure::RateLimitExceeded {
            details: message.to_string(),
            retry_delay: None,
        },
        _ if (500..=599).contains(&status) => {
            UpstreamFailure::ServerError(format!("Server error ({}): {}", status, message))
        }
        _ => UpstreamFailure::RequestFailed(format!(
            "Request failed with status {}: {}",
            status, message
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// 记录调用次数与每次 sleep 的时长；sleep 本体不真等，退避序列可精确断言。
    struct Recorder {
        calls: AtomicUsize,
        sleeps: Mutex<Vec<Duration>>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                sleeps: Mutex::new(Vec::new()),
            }
        }

        fn next_call(&self) -> usize {
            self.calls.fetch_add(1, Ordering::Relaxed) + 1
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }

        fn recorded_sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }
    }

    // ===== 哪些失败值得重试 =====

    /// 5xx 会重试，第二次成功即返回。
    /// 改坏哪行会红：`should_retry` 的 `ServerError(_) => true` 改成 `false`
    /// （或 executor 的 `attempts < policy.max_retries` 改成 `== 0`），
    /// calls 会停在 1，两条断言全红。
    #[tokio::test]
    async fn a_5xx_is_retried_then_succeeds() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::ServerError(
                        "Server error (503): upstream down".into(),
                    ))
                } else {
                    Ok("ok")
                })
            },
            |d| {
                recorder.sleeps.lock().unwrap().push(d);
                std::future::ready(())
            },
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok("ok"));
        assert_eq!(recorder.calls(), 2, "5xx 必须重试一次后成功");
        assert_eq!(recorder.recorded_sleeps().len(), 1, "重试前必须退避一次");
    }

    /// 请求超时（goose 归入 `NetworkError`，`errors.rs:141-163`）会重试。
    /// 改坏哪行会红：`should_retry` 的 `NetworkError(_) => true` 改成 `false`，
    /// calls 会停在 1。
    #[tokio::test]
    async fn a_request_timeout_is_retried() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::NetworkError(
                        "Request timed out — check your network connection and try again.".into(),
                    ))
                } else {
                    Ok(7)
                })
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok(7));
        assert_eq!(recorder.calls(), 2, "超时必须重试");
    }

    /// 连接错误（goose `errors.rs:145-158` 的 "Could not connect to ..."）会重试。
    /// 改坏哪行会红：同 `NetworkError` 分支。
    #[tokio::test]
    async fn a_connection_error_is_retried() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::NetworkError(
                        "Could not connect to 127.0.0.1:8080 — check your network connection"
                            .into(),
                    ))
                } else {
                    Ok(())
                })
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok(()));
        assert_eq!(recorder.calls(), 2, "连接错误必须重试");
    }

    /// 429 会重试。
    /// 改坏哪行会红：`should_retry` 的 `RateLimitExceeded { .. } => true`
    /// 改成 `false`，calls 停在 1。
    #[tokio::test]
    async fn a_429_is_retried() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::RateLimitExceeded {
                        details: "rate limited".into(),
                        retry_delay: None,
                    })
                } else {
                    Ok("ok")
                })
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok("ok"));
        assert_eq!(recorder.calls(), 2, "429 必须重试");
    }

    /// 429 带服务端提示时，退避用它而不是指数退避
    /// （goose `retry.rs:142-148`）。
    /// 改坏哪行会红：`retry_operation` 里 `retry_delay: Some(d) => *d` 那一臂
    /// 删掉（落回 `delay_for_attempt`），slept 变成默认的 1s，断言 7s 红。
    #[tokio::test]
    async fn a_429_retry_delay_overrides_the_exponential_backoff() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::RateLimitExceeded {
                        details: "rate limited".into(),
                        retry_delay: Some(Duration::from_secs(7)),
                    })
                } else {
                    Ok(())
                })
            },
            |d| {
                recorder.sleeps.lock().unwrap().push(d);
                std::future::ready(())
            },
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok(()));
        assert_eq!(
            recorder.recorded_sleeps(),
            vec![Duration::from_secs(7)],
            "429 的 retry_delay 必须覆盖指数退避"
        );
    }

    /// 4xx 在**接线用的策略**（`transient_only`，goose 流式路径也是这么配的，
    /// `reply_parts.rs:416`）下不重试：一次调用、零退避、错误原样返回。
    /// 改坏哪行会红：`should_retry` 里 `RequestFailed(_) => !policy.transient_only`
    /// 的 `!` 去掉（或本测试构造策略时去掉 `.transient_only()`），calls 变 2。
    #[tokio::test]
    async fn a_4xx_is_not_retried_under_the_wiring_policy() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default().transient_only(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::RequestFailed(
                        "Bad request (400): model not found".into(),
                    ))
                } else {
                    Ok(())
                })
            },
            |d| {
                recorder.sleeps.lock().unwrap().push(d);
                std::future::ready(())
            },
            || 0.5,
        )
        .await;

        assert!(matches!(result, Err(UpstreamFailure::RequestFailed(_))));
        assert_eq!(recorder.calls(), 1, "4xx 不重试：只允许一次调用");
        assert!(
            recorder.recorded_sleeps().is_empty(),
            "4xx 不重试就不该有任何退避"
        );
    }

    /// goose 的**默认**策略（`transient_only: false`）确实会重试普通 400 ——
    /// 这不是 quill 发明的放宽，而是照抄的默认值（`retry.rs:41`、`:113`；
    /// goose 自己的测试 `:277-281` 钉死了这条）。所以接线必须显式
    /// `.transient_only()`，否则 400 也会被重试。
    /// 改坏哪行会红：`RetryPolicy::default` 里 `transient_only: false` 改成
    /// `true`，calls 变 1。
    #[tokio::test]
    async fn the_goose_default_policy_does_retry_a_generic_400() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                let n = recorder.next_call();
                std::future::ready(if n == 1 {
                    Err(UpstreamFailure::RequestFailed(
                        "Bad request (400): model not found".into(),
                    ))
                } else {
                    Ok(())
                })
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok(()));
        assert_eq!(recorder.calls(), 2, "goose 默认会重试 RequestFailed");
    }

    /// 401 不重试（goose `retry.rs:351-357` 的测试同样钉死）。
    /// 改坏哪行会红：`should_retry` 的 `Authentication(_) => false` 改成 `true`，
    /// calls 变 2。
    #[tokio::test]
    async fn an_auth_error_is_never_retried() {
        let recorder = Recorder::new();
        let result: Result<(), UpstreamFailure> = retry_operation(
            &RetryPolicy::default(),
            || {
                recorder.next_call();
                std::future::ready(Err(UpstreamFailure::Authentication(
                    "Authentication failed (401): invalid key".into(),
                )))
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert!(matches!(result, Err(UpstreamFailure::Authentication(_))));
        assert_eq!(recorder.calls(), 1, "401 重试没有意义");
    }

    /// 永久性 400（thinking 块不可修改）即使走 goose 默认策略也不重试
    /// （goose `retry.rs:112` + marker 列表 `:94-98`）。
    /// 改坏哪行会红：`is_permanent_request_failure` 的 marker 列表清空
    /// （或去掉 `should_retry` 里那条 guard），calls 变 2。
    #[tokio::test]
    async fn a_permanent_thinking_block_400_is_not_retried_even_by_default() {
        let recorder = Recorder::new();
        let result: Result<(), UpstreamFailure> = retry_operation(
            &RetryPolicy::default(),
            || {
                recorder.next_call();
                std::future::ready(Err(UpstreamFailure::RequestFailed(
                    "Bad request (400): messages.3.content.1: `thinking` or `redacted_thinking` \
                     blocks in the latest assistant message cannot be modified"
                        .into(),
                )))
            },
            |_| std::future::ready(()),
            || 0.5,
        )
        .await;

        assert!(matches!(result, Err(UpstreamFailure::RequestFailed(_))));
        assert_eq!(recorder.calls(), 1, "永久性失败重试不可能成功");
    }

    // ===== 上限与放弃 =====

    /// 超过 `max_retries` 后放弃，并保留**最后一次**错误。
    /// 改坏哪行会红：executor 里 `attempts < policy.max_retries` 改成 `<=`，
    /// calls 变 4；或把 `return Err(error)` 换成 `return Err(...)` 里的旧错误，
    /// 文案断言红。
    #[tokio::test]
    async fn gives_up_after_max_retries_and_keeps_the_last_error() {
        let recorder = Recorder::new();
        let policy = RetryPolicy::new(2, 1_000, 2.0, 30_000);
        let result: Result<(), UpstreamFailure> = retry_operation(
            &policy,
            || {
                let n = recorder.next_call();
                std::future::ready(Err(UpstreamFailure::ServerError(format!("boom-{n}"))))
            },
            |d| {
                recorder.sleeps.lock().unwrap().push(d);
                std::future::ready(())
            },
            || 0.5,
        )
        .await;

        assert_eq!(
            result,
            Err(UpstreamFailure::ServerError("boom-3".into())),
            "放弃时保留最后一次错误"
        );
        assert_eq!(recorder.calls(), 3, "1 次原始 + 2 次重试");
        assert_eq!(
            recorder.recorded_sleeps(),
            vec![Duration::from_secs(1), Duration::from_secs(2)],
            "两次退避按 1s、2s 指数增长"
        );
    }

    /// 成功即停：第一次就成功不允许有任何重试或退避。
    /// 改坏哪行会红：executor 的开头 `Ok(result) => return Ok(result)` 改成
    /// `continue`（或先 sleep 再判断），calls 会大于 1。
    #[tokio::test]
    async fn success_on_the_first_try_never_sleeps() {
        let recorder = Recorder::new();
        let result = retry_operation(
            &RetryPolicy::default(),
            || {
                recorder.next_call();
                std::future::ready(Ok("done"))
            },
            |d| {
                recorder.sleeps.lock().unwrap().push(d);
                std::future::ready(())
            },
            || 0.5,
        )
        .await;

        assert_eq!(result, Ok("done"));
        assert_eq!(recorder.calls(), 1, "成功一次即停止，不许多试");
        assert!(recorder.recorded_sleeps().is_empty(), "成功时不该退避");
    }

    // ===== 退避怎么算 =====

    /// 第 0 次（不重试）没有退避（goose `retry.rs:72-74`）。
    /// 改坏哪行会红：删掉 `delay_for_attempt` 的 `if attempt == 0` 早退，
    /// 下一行 `attempt - 1` 在 usize 上下溢（debug 下 panic），测试红。
    #[test]
    fn attempt_zero_means_no_delay() {
        assert_eq!(
            RetryPolicy::default().delay_for_attempt(0, 0.5),
            Duration::ZERO
        );
    }

    /// 退避序列单调不减，且被 `max_interval_ms` 截断后不再增长。
    ///
    /// 抖动注入固定值 0.5 → 因子恰好 1.0（实机验证），所以这里能直接对出
    /// 基数序列；期望 `[0, 1s, 2s, 4s, 8s, 8s, 8s, 8s]`，后四步持平说明
    /// 上限咬住了基数（goose `retry.rs:80`）。
    ///
    /// 改坏哪行会红：删掉 `std::cmp::min(base_delay_ms, self.max_interval_ms)`
    /// 的截断，第 5 次起变成 16s/32s…，数组断言与上限断言同时红。
    #[test]
    fn backoff_is_monotonic_and_stops_at_the_cap() {
        let policy = RetryPolicy::new(7, 1_000, 2.0, 8_000);
        let observed: Vec<Duration> = (0..=7)
            .map(|attempt| policy.delay_for_attempt(attempt, 0.5))
            .collect();

        assert_eq!(
            observed,
            [0, 1_000, 2_000, 4_000, 8_000, 8_000, 8_000, 8_000]
                .iter()
                .map(|ms| Duration::from_millis(*ms))
                .collect::<Vec<_>>()
        );
        for window in observed.windows(2) {
            assert!(
                window[0] <= window[1],
                "退避必须单调不减：{:?} > {:?}",
                window[0],
                window[1]
            );
        }
        for delay in &observed {
            assert!(
                *delay <= Duration::from_millis(policy.max_interval_ms),
                "退避基数不得超过上限：{:?}",
                delay
            );
        }
    }

    /// 抖动落在 goose 的合法区间 `[0.8, 1.2] × 截断后基数`
    /// （goose `retry.rs:82-84`）。
    /// 改坏哪行会红：`JITTER_SPAN` 0.4 改 0.8，上界 36s 变 42s，断言红。
    #[test]
    fn jitter_stays_inside_the_goose_band() {
        let policy = RetryPolicy::new(3, 1_000, 2.0, 30_000);

        // 第 6 次：基数 32000ms 被截到 30000ms，再乘抖动。
        assert_eq!(
            policy.delay_for_attempt(6, 0.0),
            Duration::from_millis(24_000)
        );
        assert_eq!(
            policy.delay_for_attempt(6, 0.5),
            Duration::from_millis(30_000)
        );
        assert_eq!(
            policy.delay_for_attempt(6, 1.0),
            Duration::from_millis(36_000)
        );

        for unit in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let delay = policy.delay_for_attempt(6, unit);
            assert!(
                delay >= Duration::from_millis(24_000) && delay <= Duration::from_millis(36_000),
                "抖动越界（unit={unit}）：{delay:?}"
            );
        }
    }

    /// 注入的抖动值越界时被防御性 clamp（goose 依赖 `rand` 永远给 `[0,1)`，
    /// 本模块对调用方契约做兜底，见模块头差异 4）。
    /// 改坏哪行会红：去掉 `jitter_unit.clamp(0.0, 1.0)` 的 clamp，
    /// unit=2.0 时因子 1.6 → 48s，断言红。
    #[test]
    fn out_of_range_jitter_units_are_clamped() {
        let policy = RetryPolicy::new(3, 1_000, 2.0, 30_000);
        assert_eq!(
            policy.delay_for_attempt(6, 2.0),
            Duration::from_millis(36_000)
        );
        assert_eq!(
            policy.delay_for_attempt(6, -1.0),
            Duration::from_millis(24_000)
        );
    }

    // ===== 429 的服务端提示 =====

    /// body 的 `retry_after_seconds` 优先于 header（goose
    /// `http_status.rs:47-57`，测试 `:618-627`）。
    /// 改坏哪行会红：`extract_retry_after` 里 body 那段删掉，结果变 5s。
    #[test]
    fn retry_after_prefers_body_seconds_over_the_header() {
        let payload = serde_json::json!({
            "error": { "metadata": { "retry_after_seconds": 22.148 } }
        });
        assert_eq!(
            extract_retry_after(Some(&payload), Some("5")),
            Some(Duration::from_secs_f64(22.148))
        );
    }

    /// body 没给就用 header 的整数秒（goose `http_status.rs:630-634`）。
    /// 改坏哪行会红：`extract_retry_after` 的 header 兜底行删掉，结果变 None。
    #[test]
    fn retry_after_falls_back_to_the_header() {
        assert_eq!(
            extract_retry_after(None, Some("17")),
            Some(Duration::from_secs(17))
        );
        let payload = serde_json::json!({ "error": { "message": "rate limited" } });
        assert_eq!(extract_retry_after(Some(&payload), None), None);
    }

    /// 非法值拒收、病态大值截到 3600s、无穷拒收
    /// （goose `http_status.rs:644-650,707-718`）。
    /// 改坏哪行会红：`retry_after_from_seconds` 去掉 `!seconds.is_finite()`
    /// 或 `seconds < 0.0` 的 guard，会 panic / 给出 Some；去掉 `.min()` 截断，
    /// 1e30 断言红。
    #[test]
    fn retry_after_bad_values_are_rejected_or_clamped() {
        assert_eq!(retry_after_from_seconds(-1.0), None);
        assert_eq!(retry_after_from_seconds(f64::NAN), None);
        assert_eq!(retry_after_from_seconds(f64::INFINITY), None);
        assert_eq!(
            retry_after_from_seconds(1e30),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(retry_after_from_header_seconds("-3"), None);
        assert_eq!(retry_after_from_header_seconds("not a number"), None);
        assert_eq!(
            retry_after_from_header_seconds("3601"),
            Some(Duration::from_secs(3600))
        );
    }

    // ===== status → 分类 =====

    /// HTTP status 分类与 goose `http_status.rs:244-312` 一致。
    /// 改坏哪行会红：把 429 分支改到 `RequestFailed`，`RetryPolicy::default()
    /// .transient_only()` 下它就不再可重试，断言与重试语义都会红。
    #[test]
    fn http_status_classification_matches_goose() {
        assert!(matches!(
            from_http_status(401, "invalid key"),
            UpstreamFailure::Authentication(_)
        ));
        assert!(matches!(
            from_http_status(403, "forbidden"),
            UpstreamFailure::Authentication(_)
        ));
        assert!(matches!(
            from_http_status(404, "no such model"),
            UpstreamFailure::RequestFailed(_)
        ));
        assert!(matches!(
            from_http_status(402, "no credits"),
            UpstreamFailure::CreditsExhausted { .. }
        ));
        assert!(matches!(
            from_http_status(413, "too large"),
            UpstreamFailure::ContextLengthExceeded(_)
        ));
        assert!(matches!(
            from_http_status(400, "model not found"),
            UpstreamFailure::RequestFailed(_)
        ));

        let rate_limited = from_http_status(429, "slow down");
        assert!(matches!(
            rate_limited,
            UpstreamFailure::RateLimitExceeded {
                retry_delay: None,
                ..
            }
        ));
        assert!(should_retry(
            &rate_limited,
            &RetryPolicy::default().transient_only()
        ));

        for status in [500, 502, 503, 504] {
            let failure = from_http_status(status, "upstream down");
            assert!(
                matches!(failure, UpstreamFailure::ServerError(_)),
                "{status} 必须归 ServerError"
            );
            assert!(should_retry(
                &failure,
                &RetryPolicy::default().transient_only()
            ));
        }
        assert!(matches!(
            from_http_status(418, "teapot"),
            UpstreamFailure::RequestFailed(_)
        ));
    }
}
