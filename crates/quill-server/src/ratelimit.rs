//! 登录端点的限流。
//!
//! **为什么控制面自带的账号锁定不够用。** `quill-control` 的 `login()` 里：
//!
//! 1. 用户名不存在（或不合法）时走 `burn_equivalent_time()` 分支，返回
//!    `CredentialsRejected`——这条路径上**没有任何账号可锁**，攻击者可以
//!    拿随机用户名无限次触发 PBKDF2，同时顺手做用户名枚举。
//! 2. 锁定检查（`locked_until`）排在 `verify_stored()` **之后**。也就是说
//!    账号「已经被锁死」时，每一次尝试照样要算满一遍 PBKDF2 才被拒。
//!
//! 所以 PBKDF2 的 CPU 成本在控制面里是无法封顶的，必须在**进 DbBridge 之前**
//! 用一道进程内的滑动窗口挡住。这个限流器就是那道窗口。
//!
//! **计数口径**：同一 `(用户名, 来源 IP)` 在滑动窗口内累计**失败**次数；
//! 成功登录清空该窗口。正常用户输错一两次不会碰到阈值。
//!
//! **不写数据库**：限流状态是纯内存的，进程重启即清空。这是刻意的取舍——
//! 账号级的持久锁定由 `quill-control` 负责，本模块只负责「把 CPU 成本按住」
//! 这件事，而重启后攻击者也拿不到任何持久凭据。若把失败计数落库，攻击者
//! 只需刷失败次数就能把库撑大，那是用一个 DoS 换另一个 DoS。
//!
//! **无界增长防护**：键是攻击者可控的（随机用户名），所以表本身必须有上界。
//! 超过 `MAX_TRACKED_KEYS` 时先清掉窗口已过期的条目；仍超限就淘汰窗口最老的那一条。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// 窗口内允许的最大失败次数。与 `quill_control::AuthPolicy::production()`
/// 的 `max_failures` 保持一致，避免出现「账号锁定文案说 15 分钟，限流却按
/// 另一个时长算」这种让人困惑的双重口径。
pub const DEFAULT_MAX_FAILURES: usize = 5;

/// 滑动窗口长度（毫秒）。同样对齐 `AuthPolicy::production()` 的 15 分钟锁定。
pub const DEFAULT_WINDOW_MS: u64 = 15 * 60 * 1000;

/// 同时追踪的键数上限。超过就淘汰，防止随机用户名把内存吃光。
pub const MAX_TRACKED_KEYS: usize = 4096;

/// 锁毒化时返回的等待秒数。宁可保守地多等，也不放行。
const POISONED_RETRY_SECS: u64 = 15;

/// 纪元以来的毫秒数。系统时钟回拨时返回 0（早于纪元的时刻），
/// 交给上层去 clamp，这里不做静默修正。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 取不到真实来源时使用的占位名。
///
/// 这是**收敛**而不是「忽略」：所有取不到 IP 的请求共享一个额度，宁可让
/// 测试/内网调用方挤在一起，也不要给它们各自一份无限额度。
const UNKNOWN_PEER: &str = "unknown-peer";

/// 提取真实对端地址，**允许取不到**。
///
/// 为什么不用 `axum::extract::ConnectInfo`：它只实现了 `FromRequestParts`，
/// 没实现 `OptionalFromRequestParts`（axum 0.8 移除了对提取器的 blanket
/// `Option<T>` 支持），所以签名里不能写 `Option<ConnectInfo<_>>`。
///
/// 缺失时的行为：返回 `None`，由 [`peer_segment`] 收敛到
/// [`UNKNOWN_PEER`]，**并**打一条一次性的告警。静默降级是不行的——那会
/// 让人以为限流按 IP 生效，实际上所有人共用一个额度。
#[derive(Debug, Clone, Copy, Default)]
pub struct PeerAddr(pub Option<std::net::SocketAddr>);

impl PeerAddr {
    pub fn get(&self) -> Option<std::net::SocketAddr> {
        self.0
    }
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let found = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0);
        if found.is_none() {
            warn_missing_connect_info();
        }
        Ok(PeerAddr(found))
    }
}

/// 「没注入 ConnectInfo」只提醒一次。这类装配错误如果每次请求都打日志，
/// 本身就会变成日志放大攻击。
fn warn_missing_connect_info() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "[ratelimit] ⚠ 请求里没有 ConnectInfo，登录限流退化为「所有来源共用一个额度」。\
             下一步：用 axum::serve(...).into_make_service_with_connect_info::<SocketAddr>() \
             启动服务；若这是嵌入式调用方，请自行把真实对端地址放进请求扩展。"
        );
    });
}

/// 决定一次登录请求的「来源」段。
///
/// 口径（顺序不可调换）：
/// 1. `trust_proxy=false`（默认）→ 只用 TCP 对端地址，**完全无视**
///    `X-Forwarded-For`。这是关键：那个头是客户端自己写的，直接信它，
///    攻击者每次换一个假 IP 就能把登录限流清零。
/// 2. `trust_proxy=true` → 优先取 `X-Forwarded-For` 的**第一段**（最靠近
///    客户端的那一跳），其次 `X-Real-IP`；都没有再退回 TCP 对端。
pub fn peer_segment(
    headers: &axum::http::HeaderMap,
    peer: Option<std::net::SocketAddr>,
    trust_proxy: bool,
) -> String {
    if trust_proxy {
        if let Some(forwarded) = header_str(headers, "x-forwarded-for") {
            if let Some(first) = forwarded.split(',').next() {
                let first = first.trim();
                if !first.is_empty() {
                    return normalize_ip(first);
                }
            }
        }
        if let Some(real) = header_str(headers, "x-real-ip") {
            if !real.trim().is_empty() {
                return normalize_ip(real.trim());
            }
        }
    }
    match peer {
        Some(addr) => normalize_ip(&addr.ip().to_string()),
        None => UNKNOWN_PEER.to_string(),
    }
}

fn header_str(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// 把来源归一成统一形态。
///
/// 代理有可能只给裸 IP，而对端地址带端口；不归一的话同一个客户端会被算成
/// 两个来源，额度翻倍。不做 IP 语法解析：限流不需要理解地址，只需要能
/// 一致地比较；解析失败就原样返回。
fn normalize_ip(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        UNKNOWN_PEER.to_string()
    } else {
        t.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitPolicy {
    pub max_failures: usize,

    pub window_ms: u64,
}

impl Default for LimitPolicy {
    fn default() -> Self {
        Self {
            max_failures: DEFAULT_MAX_FAILURES,
            window_ms: DEFAULT_WINDOW_MS,
        }
    }
}

#[derive(Debug, Default)]
struct Window {
    /// 窗口内的失败时刻，**升序**。队首是最老的一条。
    failures: VecDeque<u64>,
}

#[derive(Debug)]
pub struct RateLimiter {
    policy: LimitPolicy,
    windows: Mutex<HashMap<String, Window>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(LimitPolicy::default())
    }
}

impl RateLimiter {
    pub fn new(policy: LimitPolicy) -> Self {
        Self {
            policy,
            windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn policy(&self) -> LimitPolicy {
        self.policy
    }

    /// 构造登录限流用的键。
    ///
    /// 用户名统一小写去空白后参与计算——否则攻击者换个大小写就能拿一份
    /// 干净的计数额度。IP 单独成段，避免 `a` + `bc` 与 `ab` + `c` 拼成同一个键。
    pub fn key(username: &str, peer: &str) -> String {
        let u = username.trim().to_lowercase();
        let p = peer.trim();
        format!("{u}\u{1}{p}")
    }

    /// 判断这个键当前**能不能**发起一次登录尝试，不做任何密码校验。
    ///
    /// 返回 `Err(等待秒数)` 表示已被限流，调用方应**立刻**返回 429，
    /// 不要再进 `DbBridge` —— 那一步才是 PBKDF2 的成本所在。
    ///
    /// `now_ms` 是**权威时钟**，由调用方传入（生产路径传
    /// [`now_ms`]）。这样窗口滑动在测试里完全确定，不必 sleep。
    pub fn acquire(&self, key: &str, now_ms: u64) -> Result<(), u64> {
        let now = now_ms;
        let Ok(mut map) = self.windows.lock() else {
            return Err(POISONED_RETRY_SECS);
        };
        let window = Self::entry(&mut map, key, self.policy.window_ms, now);
        Self::prune(window, now, self.policy.window_ms);

        if window.failures.len() >= self.policy.max_failures {
            let oldest = window.failures.front().copied().unwrap_or(now);
            let recover_at = oldest.saturating_add(self.policy.window_ms);
            // +1 是为了让「等满窗口后第一次重试」一定落在窗口之外，
            // 否则客户端会拿到 Retry-After: 0 然后立刻再撞一次。
            let wait = recover_at.saturating_sub(now).saturating_add(1);
            return Err((wait / 1000).max(1));
        }
        Ok(())
    }

    /// 记一次失败。
    ///
    /// 阈值是「窗口内第 N 次失败之后开始拦」，所以第 N 次失败本身已经把
    /// PBKDF2 烧掉了，从第 N+1 次起才 429。这是滑动窗口计数器的固有
    /// 代价：最多多烧一轮。想提前一轮就得在 `acquire` 时预扣额度，但那会把
    /// 「正常但输错」也算成扣款，手慢的真人会被误伤，不划算。
    pub fn record_failure(&self, key: &str, now_ms: u64) {
        let now = now_ms;
        let Ok(mut map) = self.windows.lock() else {
            return;
        };
        let window = Self::entry(&mut map, key, self.policy.window_ms, now);
        Self::prune(window, now, self.policy.window_ms);
        if window.failures.len() >= self.policy.max_failures {
            return;
        }
        window.failures.push_back(now);
    }

    /// 成功登录：清空该窗口。真人输对了一次就说明不是爆破，之前的失败不该
    /// 继续拖累他（换 IP、换浏览器也走这里）。
    pub fn record_success(&self, key: &str) {
        let Ok(mut map) = self.windows.lock() else {
            return;
        };
        if let Some(w) = map.get_mut(key) {
            w.failures.clear();
        }
    }

    /// 仅供测试与诊断：当前窗口内的失败次数。
    pub fn failure_count(&self, key: &str, now_ms: u64) -> usize {
        let Ok(mut map) = self.windows.lock() else {
            return 0;
        };
        let Some(w) = map.get_mut(key) else {
            return 0;
        };
        Self::prune(w, now_ms, self.policy.window_ms);
        w.failures.len()
    }

    /// 仅供测试与诊断：当前追踪的键数。
    pub fn tracked_keys(&self) -> usize {
        self.windows.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// 仅供测试：清空全部状态。
    pub fn reset(&self) {
        if let Ok(mut map) = self.windows.lock() {
            map.clear();
        }
    }

    /// 取到（必要时建出）一个窗口，并在必要时为无界增长兜底。
    fn entry<'a>(
        map: &'a mut HashMap<String, Window>,
        key: &str,
        window_ms: u64,
        now: u64,
    ) -> &'a mut Window {
        if !map.contains_key(key) {
            if map.len() >= MAX_TRACKED_KEYS {
                Self::make_room(map, window_ms, now);
            }
            map.insert(key.to_string(), Window::default());
        }
        map.get_mut(key).expect("上一步刚确保过键存在")
    }

    /// 键数超上界时的腾挪策略：先删窗口已完全过期的；都还在有效期的话，
    /// 淘汰窗口最老的那一条（离恢复最近，离攻击价值也最低）。
    fn make_room(map: &mut HashMap<String, Window>, window_ms: u64, now: u64) {
        map.retain(|_, w| {
            Self::prune(w, now, window_ms);
            !w.failures.is_empty()
        });
        if map.len() < MAX_TRACKED_KEYS {
            return;
        }
        let oldest = map
            .iter()
            .min_by_key(|(_, w)| w.failures.front().copied().unwrap_or(u64::MAX))
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            map.remove(&k);
        }
    }

    /// 丢掉滑出窗口的失败记录。保留后缀，不动其余条目顺序。
    fn prune(w: &mut Window, now: u64, window_ms: u64) {
        let cutoff = now.saturating_sub(window_ms);
        while w.failures.front().is_some_and(|t| *t <= cutoff) {
            w.failures.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定 `now_ms` 的构造方式：直接操作内部结构，绕开系统时钟，
    /// 让窗口滑动的断言完全确定。
    fn limiter(policy: LimitPolicy) -> RateLimiter {
        RateLimiter::new(policy)
    }

    fn quick(max_failures: usize, window_ms: u64) -> RateLimiter {
        limiter(LimitPolicy {
            max_failures,
            window_ms,
        })
    }

    #[test]
    fn default_policy_matches_the_control_plane_account_lock() {
        // 两处口径必须一致：账号锁定说 15 分钟，限流却是别的时长，
        // 用户会看到互相矛盾的提示。
        let p = LimitPolicy::default();
        assert_eq!(p.max_failures, DEFAULT_MAX_FAILURES);
        assert_eq!(p.window_ms, 15 * 60 * 1000);
    }

    #[test]
    fn username_case_and_padding_do_not_split_the_budget() {
        assert_eq!(RateLimiter::key(" Alice ", "1.2.3.4"), RateLimiter::key("alice", "1.2.3.4"));
        assert_ne!(RateLimiter::key("alice", "1.2.3.4"), RateLimiter::key("bob", "1.2.3.4"));
    }

    #[test]
    fn the_separator_cannot_be_forged_into_a_key_collision() {
        // 分隔符用 \u{1}（不可能出现在用户名或 IP 里）。如果用普通字符，
        // ("ab","c") 与 ("a","bc") 会拼成同一个键。
        assert_ne!(RateLimiter::key("ab", "c"), RateLimiter::key("a", "bc"));
    }

    #[test]
    fn the_window_blocks_once_the_failure_budget_is_spent() {
        let rl = quick(3, 60_000);
        let k = RateLimiter::key("alice", "10.0.0.1");

        assert!(rl.acquire(&k, 1_000).is_ok());
        rl.record_failure(&k, 1_000);
        assert!(rl.acquire(&k, 1_100).is_ok());
        rl.record_failure(&k, 1_100);
        assert!(rl.acquire(&k, 1_200).is_ok());
        rl.record_failure(&k, 1_200);

        let blocked = rl.acquire(&k, 1_300).expect_err("额度用尽后必须挡住");
        assert!(blocked >= 1, "Retry-After 不能是 0 秒，否则客户端立刻重试");
    }

    #[test]
    fn old_failures_slide_out_of_the_window_instead_of_blocking_forever() {
        let rl = quick(2, 1_000);
        let k = RateLimiter::key("bob", "10.0.0.2");

        rl.record_failure(&k, 10_000);
        rl.record_failure(&k, 10_000);
        assert!(rl.acquire(&k, 10_000).is_err(), "窗口内必须挡住");

        // 越过窗口后，早先的失败不再计数。
        assert!(
            rl.acquire(&k, 10_000 + 1_000 + 1).is_ok(),
            "窗口滑过之后必须放行，否则用户会被永久锁死"
        );
        assert_eq!(rl.failure_count(&k, 10_000 + 1_000 + 1), 0);
    }

    #[test]
    fn a_successful_login_clears_the_budget() {
        let rl = quick(2, 60_000);
        let k = RateLimiter::key("carol", "10.0.0.3");

        rl.record_failure(&k, 1_000);
        rl.record_failure(&k, 1_000);
        assert!(rl.acquire(&k, 1_000).is_err());

        rl.record_success(&k);
        assert_eq!(rl.failure_count(&k, 1_000), 0);
        assert!(
            rl.acquire(&k, 1_000).is_ok(),
            "输对一次之后不该继续被之前的失败拖累"
        );
    }

    #[test]
    fn different_usernames_and_ips_have_independent_budgets() {
        let rl = quick(1, 60_000);
        let a = RateLimiter::key("dave", "10.0.0.4");
        let b = RateLimiter::key("eve", "10.0.0.4");
        let c = RateLimiter::key("dave", "10.0.0.5");

        rl.record_failure(&a, 1_000);
        assert!(rl.acquire(&a, 1_000).is_err());
        assert!(rl.acquire(&b, 1_000).is_ok(), "换个用户名有独立额度");
        assert!(rl.acquire(&c, 1_000).is_ok(), "换个 IP 有独立额度");    }

    #[test]
    fn tracking_stays_bounded_when_usernames_are_random() {
        // 攻击者拿随机用户名打过来，键数不能跟着请求数线性涨——那等于
        // 把限流器本身变成内存耗尽的入口。
        let rl = limiter(LimitPolicy::default());
        for i in 0..(MAX_TRACKED_KEYS * 3) {
            let k = RateLimiter::key(&format!("user-{i}"), "10.0.0.9");
            if rl.acquire(&k, 1_000).is_ok() {
                rl.record_failure(&k, 1_000);
            }
        }
        assert!(
            rl.tracked_keys() <= MAX_TRACKED_KEYS,
            "追踪的键数必须封顶，实际 {} 条",
            rl.tracked_keys()
        );
    }

    #[test]
    fn a_fully_expired_table_is_emptied_before_any_eviction() {
        let rl = limiter(LimitPolicy::default());
        let t = 1_000_000_000u64;
        {
            let mut map = rl.windows.lock().expect("锁可用");
            for i in 0..(MAX_TRACKED_KEYS + 50) {
                let w = map.entry(format!("old-{i}")).or_default();
                w.failures.push_back(0); // 纪元时刻，必然已滑出窗口
            }
        }
        // 用一个远超窗口的 now 触发清理。
        let k = RateLimiter::key("newcomer", "10.0.0.10");
        assert!(rl.acquire(&k, t).is_ok());
        assert!(
            rl.tracked_keys() < MAX_TRACKED_KEYS,
            "过期条目应被整批清掉，实际 {} 条",
            rl.tracked_keys()
        );
    }

    #[test]
    fn retry_hint_never_tells_the_client_to_come_back_immediately() {
        let rl = quick(1, 1_000);
        let k = RateLimiter::key("frank", "10.0.0.11");
        rl.record_failure(&k, 1_000);
        let wait = rl.acquire(&k, 1_000).expect_err("已超阈值");
        assert!(wait >= 1, "Retry-After 必须是正秒数，实际 {wait}");
    }

    fn headers(pairs: &[(&'static str, &str)]) -> axum::http::HeaderMap {
        let mut m = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            m.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).expect("合法头名"),
                axum::http::HeaderValue::from_str(v).expect("合法头值"),
            );
        }
        m
    }

    fn peer(s: &str) -> std::net::SocketAddr {
        s.parse().expect("合法地址")
    }

    #[test]
    fn a_forged_forwarded_for_header_is_ignored_by_default() {
        // 这是整个限流能否成立的前提：默认不信 XFF。否则攻击者每次带一个
        // 新的假 IP，限流窗口永远是空的，等于没有限流。
        let h = headers(&[("x-forwarded-for", "1.2.3.4")]);
        let seg = peer_segment(&h, Some(peer("10.0.0.5:5555")), false);
        assert_eq!(seg, "10.0.0.5", "默认必须用真实对端地址，而不是客户端自填的头");
    }

    #[test]
    fn trusting_the_proxy_takes_the_closest_hop_of_the_forwarded_chain() {
        let h = headers(&[("x-forwarded-for", "203.0.113.9, 10.1.1.1, 10.2.2.2")]);
        let seg = peer_segment(&h, Some(peer("10.0.0.5:5555")), true);
        assert_eq!(
            seg, "203.0.113.9",
            "XFF 是一串代理依次追加的，最左边才是最初的客户端"
        );
    }

    #[test]
    fn trusted_real_ip_is_used_when_forwarded_for_is_absent() {
        let h = headers(&[("x-real-ip", "198.51.100.7")]);
        let seg = peer_segment(&h, Some(peer("10.0.0.5:5555")), true);
        assert_eq!(seg, "198.51.100.7");
    }

    #[test]
    fn trusting_the_proxy_still_falls_back_to_the_peer_when_headers_are_empty() {
        let seg = peer_segment(&headers(&[]), Some(peer("10.0.0.5:5555")), true);
        assert_eq!(seg, "10.0.0.5", "代理没加头时不能把来源判成未知");
    }

    #[test]
    fn an_empty_forwarded_chain_falls_through_instead_of_becoming_an_empty_source() {
        // 空段会让所有这类请求挤进同一个空字符串键，或更糟——各自新建一份。
        let h = headers(&[("x-forwarded-for", "  ,  "), ("x-real-ip", "  ")]);
        let seg = peer_segment(&h, Some(peer("10.0.0.5:5555")), true);
        assert_eq!(seg, "10.0.0.5");
    }

    #[test]
    fn requests_without_any_known_source_share_one_budget_rather_than_getting_unlimited_ones() {
        let seg = peer_segment(&headers(&[]), None, false);
        assert_eq!(seg, UNKNOWN_PEER);
    }
}
