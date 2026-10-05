//! 随机源、十六进制编解码、SHA-256 摘要。
//!
//! # 生产熵源是真 CSPRNG（[`OsEntropySource`]），来源是 `getrandom(2)`
//!
//! 会话令牌与邀请码的**不可猜测性是安全属性**，所以熵源必须是操作系统内核熵池。
//! [`OsEntropySource`] 直接调用 `getrandom::fill`（Linux 上即 `getrandom(2)` 系统调用）。
//!
//! # 🔴 熵源失败的处理：**panic**，且不提供任何回退
//!
//! `getrandom` 失败**绝不**降级成弱随机 —— 那等于把这个修复要堵的洞原样请回来。
//! 实现在 [`fill_or_panic`]，失败时 [`entropy_unavailable_message`] 给出人话原因。
//!
//! 为什么 panic 而不是返回 `Result`：
//!
//! - [`SecretSource::fill`] 的签名返回 `()`，改成 `Result` 会把这个错误
//!   传染到**每一个**调用点（`random_bytes` / `new_session_token` /
//!   `new_invite_code` / 密码盐 / 16 字节标识），是一次与本修复无关的大范围签名变更；
//! - 熵源不可用时**没有任何安全的替代值**。继续跑意味着签发一个可被推算的令牌，
//!   那比进程停下来**危险得多**（前者静默、后者响亮）。
//!
//! ⚠️ 铁律七的「配置错误不得 panic」在此**不适用**，因为这不是配置问题：
//! 环境变量指错可以回退默认 + WARN，而**内核熵池取不到数**没有可回退的目标。
//! 措辞刻意把它与配置错误区分开（见 [`entropy_unavailable_message`]），
//! 避免值班的人按「改配置」的方向排查。行为由测试
//! `os_entropy_source_panics_when_the_os_entropy_source_fails` 钉住。
//!
//! # [`NonCsprngEntropySource`] 已降级为 `#[cfg(test)]`
//!
//! 它由 [`os_seed`] 派生：后者把 `RandomState` 的**进程级 128 位密钥**
//! 与一个单调推进的计数器哈希一下取出。所以：
//!
//! - 每次取值都不同（计数器 + 纳秒在变）→ 表面上「看起来是随机的」；
//! - 但整个进程生命周期内，令牌的总熵**上限就是那 128 位**，
//!   而计数器与纳秒都可被调用方观测或猜到；
//! - 于是拿到一个令牌的人**无法被排除**推算出同进程内的相邻令牌。
//!
//! 逐条断言见 `tests::the_fake_source_derives_everything_from_one_process_level_secret`
//! （该测试的文档里记着一处「初版断言写错、被测试当场打脸」的教训）。
//!
//! 主理人裁决 `D-2026-10-05-08` 要求保留它但**禁止生产路径引用**，
//! 所以它只在本 crate 的单元测试（`#[cfg(test)]`）下编译 ——
//! 误用会在**编译期**就断掉，而不是等到评审发现。
//!
//! # 令牌与邀请码的存储口径
//!
//! 两者都**只存摘要，不存原文**（`sessions_auth.token_hash` / `invites.code_hash`
//! 均为 32 字节，schema 里有 `CHECK(length(...) = 32)`）。
//! 摘要算法是 `SHA-256(令牌 ASCII 字节)`：
//!
//! - 令牌由 32 字节 CSPRNG 输出构成（256 位熵），
//!   所以摘要不需要再当 KDF 用 —— 抗暴力枚举靠的是**令牌本身熵足够**，
//!   不是靠哈希慢。这与密码必须用慢 KDF（见 [`password`](crate::password)）是不同的理由。

// ⚠️ 下面两个 import 只被 `#[cfg(test)]` 的 NonCsprngEntropySource 使用。
// 生产构建里它们不存在 —— 这一点由 `cargo build`（非 test）证明：
// 若误留在非 test 路径，unused import 会被 clippy -D warnings 判红。
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::error::ControlError;

/// 会话令牌原始字节数（32 字节 = 256 位熵）。
pub const TOKEN_BYTES: usize = 32;

/// 会话令牌的十六进制长度（64 字符）。
pub const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// 邀请码原始字节数（16 字节 = 128 位熵）。
pub const INVITE_CODE_BYTES: usize = 16;

/// 邀请码的十六进制长度（32 字符）。
pub const INVITE_CODE_HEX_LEN: usize = INVITE_CODE_BYTES * 2;

/// 随机源。
///
/// 抽成 trait 的唯一理由：**可测**。测试注入确定性源，
/// 于是「令牌摘要」「邀请码摘要」这些断言不依赖随机值，
/// 失败时能稳定复现（这是 AGENTS.md 铁律十三「相邻形态行为一致」的前提）。
pub trait SecretSource: Send + Sync + 'static {
    /// 填满 `out`（**必须**填满：返回后 `out` 每一字节都被覆盖）。
    ///
    /// # 调用方无法校验这一点 —— 这里是**契约**不是可执行断言
    ///
    /// `random_bytes` 只能看到「`out` 有多长」，看不到「实现写了几个字节」，
    /// 所以它**没有**、也**不假装有**违约检测（写一个恒真的断言就是假闸门，
    /// 属 `AGENTS.md` 八类失效里最容易骗过人的一种）。
    /// 因此「填满」靠实现者自觉 + 本模块内所有 [`SecretSource`] 实现的代码评审保证。
    fn fill(&self, out: &mut [u8]);
}

/// ✅ **密码学安全**的随机源：直接取操作系统内核熵池（`getrandom`）。
///
/// Linux 上 `getrandom::fill` 就是 `getrandom(2)` 系统调用，
/// 没有用户态状态、没有计数器、没有时间输入 —— 拿到一个输出
/// **推不出**前一个或后一个输出，这是 [`NonCsprngEntropySource`] 做不到的。
///
/// 无状态：`Default` 与 `new()` 等价，克隆没有意义，故不实现 `Clone`。
#[derive(Debug, Default)]
pub struct OsEntropySource;

/// 熵源不可用时的 `panic!` 消息。
///
/// 措辞刻意说清「**这不是配置问题**」：值班的人第一反应会是去翻环境变量，
/// 而这里要修的是内核 / 容器 / seccomp 限制，翻配置是白费时间（铁律七的精神）。
pub fn entropy_unavailable_message(err: &getrandom::Error) -> String {
    format!(
        "系统熵源不可用：getrandom 失败（{err}）。\
         这不是配置问题，排查环境变量没有意义——内核熵池取不到随机数时没有安全的替代值。\
         请检查宿主机内核版本 / 容器是否限制了 getrandom(2) / seccomp 是否拦截该系统调用。\
         进程终止是预期行为：继续运行只会签发可被推算的会话令牌。"
    )
}

/// 用 `getrandom` 填满 `out`；失败则 panic（**绝不**回退到弱随机）。
///
/// 抽成独立函数是为了**可测**：真机上 `getrandom` 几乎不会失败，
/// 所以失败路径由 [`tests::os_entropy_source_panics_when_the_os_entropy_source_fails`]
/// 通过注入一个必然失败的读取器来钉住。
fn fill_or_panic(out: &mut [u8], read: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>) {
    if let Err(e) = read(out) {
        panic!("{}", entropy_unavailable_message(&e));
    }
}

impl SecretSource for OsEntropySource {
    fn fill(&self, out: &mut [u8]) {
        // 空切片无需向内核要熵；直接返回，否则零长度系统调用在个别平台上是无意义的失败。
        if out.is_empty() {
            return;
        }
        fill_or_panic(out, getrandom::fill);
    }
}

/// 🔴 **非**密码学安全的随机源 —— **仅限测试**（`#[cfg(test)]`，生产路径编译不到）。
///
/// 主理人裁决 `D-2026-10-05-08`：本类型**保留**但降级为测试专用。
/// 它由 [`os_seed`] 的每进程 128 位种子 + 计数器 + 纳秒混合而成，
/// 拿到一个令牌的人无法被排除推算出同进程内的相邻令牌。
///
/// `#[cfg(test)]` 打在**类型定义**上（不是仅文档警告），所以任何生产代码
/// 一旦引用它就是**编译错误** —— 误用在编译期断掉。
#[cfg(test)]
#[derive(Debug)]
pub struct NonCsprngEntropySource {
    counter: AtomicU64,
    seed: [u64; 2],
}

#[cfg(test)]
impl NonCsprngEntropySource {
    /// 构造：从 `RandomState` 取每进程 128 位种子。
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            seed: os_seed(),
        }
    }
}

#[cfg(test)]
impl Default for NonCsprngEntropySource {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 `RandomState` 榨出 128 位：进程级密钥 + 单调推进的 k0 的哈希。
///
/// ⚠️ `RandomState` 的内部键**不可读**，只能用「哈希一个已知常量、看结果」的方式取。
/// 这不是绕过限制，而是 std 唯一提供的 OS 熵入口。
///
/// ⚠️ **精确口径**（初版文档写错过，被 `cargo test` 打脸后更正）：
/// 返回值**不是**「每进程固定不变」的那一份 —— `RandomState::new()` 会推进
/// 线程局部的 k0，所以两次调用得到**不同**的值。
/// 真正可推导的弱点在于：这些值全部派生自**同一个进程级 128 位密钥**，
/// 因此进程内所有令牌的**总熵上限是那 128 位**，而不是每令牌 256 位。
///
/// ⚠️ 本函数只服务于 [`NonCsprngEntropySource`]，而后者是 `#[cfg(test)]`；
/// 因此本函数也一并 `#[cfg(test)]`，免得死代码掩盖真正的缺口。
#[cfg(test)]
fn os_seed() -> [u64; 2] {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

    // 两次 `RandomState::new()` 的 k0 递增（k1 不变），所以两个哈希值彼此不同，
    // 且都受 k0/k1 这 128 位隐藏种子影响。
    [
        RandomState::new().hash_one(0xA5A5_5A5Au64),
        RandomState::new().hash_one(0x5A5A_5A5Au64),
    ]
}

#[cfg(test)]
impl SecretSource for NonCsprngEntropySource {
    fn fill(&self, out: &mut [u8]) {
        let mut written = 0usize;
        while written < out.len() {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            // 输入布局固定：8 字节计数器 ‖ 8 字节纳秒 ‖ 2×8 字节进程种子
            let mut block = [0u8; 32];
            block[0..8].copy_from_slice(&n.to_le_bytes());
            block[8..16].copy_from_slice(&nanos.to_le_bytes());
            block[16..24].copy_from_slice(&self.seed[0].to_le_bytes());
            block[24..32].copy_from_slice(&self.seed[1].to_le_bytes());
            let digest = sha256(&block);
            let take = (out.len() - written).min(digest.len());
            out[written..written + take].copy_from_slice(&digest[..take]);
            written += take;
        }
    }
}

/// 从随机源取 `n` 字节。
///
/// ⚠️ 本函数**不校验** [`SecretSource::fill`] 是否真的填满了 `out` ——
/// 那是一个无法从 `out` 观察的性质。与其写一条恒真的断言冒充闸门，
/// 不如把该义务写进 trait 文档（见 `fill` 的说明）。
pub fn random_bytes(src: &dyn SecretSource, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    src.fill(&mut out);
    out
}

/// SHA-256 摘要。
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// 小写十六进制编码。
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(HEX[(b >> 4) as usize]));
        s.push(char::from(HEX[(b & 0x0f) as usize]));
    }
    s
}

/// 严格解析**恰好 `2 * N` 位小写十六进制**为 `N` 字节。
///
/// **严格**（只接受小写、长度必须精确匹配）而不是「宽松清洗」：
/// 宽松解析会让「令牌大写 / 带横线」这类输入静默通过，
/// 而令牌格式错就该是 401，而不是「被悄悄纠正后仍然有效」——
/// 后者意味着同一个凭据有无穷多种写法，审计日志无法复现。
pub fn hex_decode_exact<const N: usize>(
    s: &str,
    malformed: fn() -> ControlError,
) -> Result<[u8; N], ControlError> {
    let b = s.as_bytes();
    if b.len() != N * 2 {
        return Err(malformed());
    }
    let mut out = [0u8; N];
    for i in 0..N {
        let hi = hex_val(b[i * 2]).ok_or_else(malformed)?;
        let lo = hex_val(b[i * 2 + 1]).ok_or_else(malformed)?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// 生成新的会话令牌（64 位小写十六进制）。
pub fn new_session_token(src: &dyn SecretSource) -> String {
    hex_encode(&random_bytes(src, TOKEN_BYTES))
}

/// 会话令牌摘要（写 `sessions_auth.token_hash` 的 32 字节）。
pub fn session_token_digest(token: &str) -> Result<[u8; 32], ControlError> {
    let raw = hex_decode_exact::<{ TOKEN_BYTES }>(token, || ControlError::TokenMalformed)?;
    // ⚠️ 摘要对象是**解码后的 32 字节**，不是 ASCII 文本。
    // 两者都无碰撞风险，但按字节摘要意味着「摘要与令牌表示形式解耦」，
    // 将来若把令牌改成 base64url 也只需改这一行。
    Ok(sha256(&raw))
}

/// 生成新的邀请码（32 位小写十六进制）。
pub fn new_invite_code(src: &dyn SecretSource) -> String {
    hex_encode(&random_bytes(src, INVITE_CODE_BYTES))
}

/// 邀请码摘要（写 `invites.code_hash` 的 32 字节）。
pub fn invite_code_digest(code: &str) -> Result<[u8; 32], ControlError> {
    let raw = hex_decode_exact::<{ INVITE_CODE_BYTES }>(code, || ControlError::InviteMalformed)?;
    Ok(sha256(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性随机源：按序产出 `0x00,0x01,0x02…`，让摘要断言可复现。
    #[derive(Debug)]
    struct CountingSource {
        next: AtomicU64,
    }

    impl SecretSource for CountingSource {
        fn fill(&self, out: &mut [u8]) {
            for slot in out.iter_mut() {
                *slot = u8::try_from(self.next.fetch_add(1, Ordering::SeqCst) & 0xff).unwrap_or(0);
            }
        }
    }

    /// 恒产出同一字节的随机源 —— 用来证明「重复调用不会撞车」是靠熵源而非靠调用方。
    #[derive(Debug, Default)]
    struct ZeroSource;

    impl SecretSource for ZeroSource {
        fn fill(&self, out: &mut [u8]) {
            out.fill(0);
        }
    }

    #[test]
    fn hex_roundtrip_is_lossless() {
        let src = CountingSource {
            next: AtomicU64::new(0),
        };
        let bytes = random_bytes(&src, TOKEN_BYTES);
        let hex = hex_encode(&bytes);
        assert_eq!(hex.len(), TOKEN_HEX_LEN, "必须是 64 位");
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        let back = hex_decode_exact::<{ TOKEN_BYTES }>(&hex, || ControlError::TokenMalformed)
            .expect("应能逆转");
        assert_eq!(back.to_vec(), bytes, "hex 编码不是可逆的");
    }

    #[test]
    fn hex_decode_rejects_wrong_length_and_uppercase() {
        let e = || ControlError::TokenMalformed;
        // 装置可信性：先证明合法输入确实能过，否则下面几条可能在测空气
        assert!(
            hex_decode_exact::<32>(&"0".repeat(64), e).is_ok(),
            "合法输入应通过"
        );
        assert_eq!(hex_decode_exact::<32>(&"0".repeat(63), e), Err(e()));
        assert_eq!(hex_decode_exact::<32>(&"0".repeat(65), e), Err(e()));
        assert_eq!(hex_decode_exact::<32>("", e), Err(e()));
        // 大写必须被拒：宽松接受会让「令牌大小写」变成两种不同的有效令牌
        assert_eq!(hex_decode_exact::<32>(&"A".repeat(64), e), Err(e()));
        let with_bad_tail = "0".repeat(63) + "g";
        assert_eq!(hex_decode_exact::<32>(&with_bad_tail, e), Err(e()));
    }

    #[test]
    fn hex_decode_exact_handles_other_widths() {
        // 邀请码是 16 字节 → 32 位 hex，与令牌走**同一段代码**（N 是 const 泛参）。
        let e = || ControlError::InviteMalformed;
        let c = "0123456789abcdef0123456789abcdef";
        assert_eq!(c.len(), INVITE_CODE_HEX_LEN);
        let b = hex_decode_exact::<{ INVITE_CODE_BYTES }>(c, e).expect("合法邀请码应能解析");
        assert_eq!(b[0], 0x01);
        assert_eq!(b[15], 0xef);
        // 33 位（令牌宽度）喂给邀请码宽度 → 必须报错，不能「截断前 32 位」
        assert_eq!(
            hex_decode_exact::<{ INVITE_CODE_BYTES }>(&"0".repeat(33), e),
            Err(e())
        );
    }

    #[test]
    fn sha256_matches_published_vector() {
        // 权威向量来自 NIST FIPS 180-4 示例 / OpenSSL 文档，可独立复核：
        // echo -n "abc" | sha256sum
        assert_eq!(
            hex_encode(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_encode(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn token_digest_is_stable_and_length_32() {
        let token = "0".repeat(64);
        let d = session_token_digest(&token).expect("格式合法");
        assert_eq!(d.len(), 32, "摘要必须正好 32 字节（schema 有 CHECK）");
        assert_eq!(
            d,
            session_token_digest(&token).expect("同上"),
            "摘要必须可复现"
        );
        // 令牌不同 → 摘要不同（否则不同令牌能互相冒用）
        let other = "0".repeat(63) + "1";
        assert_ne!(d, session_token_digest(&other).expect("格式合法"));
        // 格式错 → 明确错误，而不是「算出个摘要蒙混过去」
        assert_eq!(
            session_token_digest("nope"),
            Err(ControlError::TokenMalformed)
        );
    }

    #[test]
    fn invite_digest_enforces_32_char_shape() {
        assert!(
            invite_code_digest(&"a".repeat(32)).is_ok(),
            "合法邀请码应通过"
        );
        assert_eq!(
            invite_code_digest(&"a".repeat(31)),
            Err(ControlError::InviteMalformed)
        );
        assert_eq!(
            invite_code_digest(&"A".repeat(32)),
            Err(ControlError::InviteMalformed),
            "大写邀请码必须被拒"
        );
    }

    #[test]
    fn random_bytes_fills_exactly_the_requested_length() {
        let src = CountingSource {
            next: AtomicU64::new(0),
        };
        for n in [0usize, 1, 16, 31, 32, 33, 64, 1000] {
            assert_eq!(random_bytes(&src, n).len(), n, "长度 {n} 不符");
        }
    }

    #[test]
    fn non_csprng_source_is_send_sync_and_differs_per_call() {
        let s = NonCsprngEntropySource::new();
        let a = random_bytes(&s, TOKEN_BYTES);
        let b = random_bytes(&s, TOKEN_BYTES);
        assert_eq!(a.len(), TOKEN_BYTES);
        assert_ne!(a, b, "同一进程内两次取值必须不同，否则令牌会撞车");
        // 跨「实例」也必须不同：两个 ControlPlane 各自 new 也不能给同一令牌
        let c = random_bytes(&NonCsprngEntropySource::new(), TOKEN_BYTES);
        assert_ne!(a, c);
    }

    #[test]
    fn zero_source_still_produces_full_length_output() {
        // 装置可信性：证明 random_bytes 不依赖「熵源有内容」才能工作 ——
        // 否则上面那条 non_csprng 测试可能只是在测「随机源不返回全零」。
        let out = random_bytes(&ZeroSource, TOKEN_BYTES);
        assert_eq!(out.len(), TOKEN_BYTES);
        assert!(out.iter().all(|b| *b == 0), "ZeroSource 应全零");
    }

    #[test]
    fn generated_token_shape_matches_schema_expectations() {
        let s = OsEntropySource;
        let t = new_session_token(&s);
        assert_eq!(t.len(), TOKEN_HEX_LEN);
        assert!(session_token_digest(&t).is_ok());
        let c = new_invite_code(&s);
        assert_eq!(c.len(), INVITE_CODE_HEX_LEN);
        assert!(invite_code_digest(&c).is_ok());
        // 两次生成必须不同
        assert_ne!(t, new_session_token(&s));
    }

    // ---- 以下是 P0 安全修复（`D-2026-10-05-08`）的验收测试 ----

    /// 生产熵源连续取 2000 次令牌，**两两不相等**。
    ///
    /// N=2000 远超要求的 1000：撞车概率不是「大概不会发生」而是可算的
    /// （2000² / 2 = 2×10⁶ 对比较，256 位输出下期望碰撞数 ≈ 10⁻⁷⁰）。
    /// 用 `HashSet` 而不是嵌套循环：后者是 O(N²) = 400 万次比较，慢且无谓。
    #[test]
    fn os_entropy_source_yields_no_duplicate_tokens_over_2000_draws() {
        const N: usize = 2000;
        let s = OsEntropySource;
        let mut seen = std::collections::HashSet::with_capacity(N);
        for i in 0..N {
            let t = new_session_token(&s);
            assert_eq!(t.len(), TOKEN_HEX_LEN, "第 {i} 次令牌长度不对");
            assert!(
                seen.insert(t.clone()),
                "第 {i} 次令牌与之前某次完全相同：{t} —— 令牌撞车即为凭据泄漏"
            );
        }
        assert_eq!(seen.len(), N, "必须真的检查了 {N} 个不同的令牌");
    }

    /// 邀请码同样来自生产熵源，也要两两不相等（它只有 128 位熵，更经不起撞车）。
    #[test]
    fn os_entropy_source_yields_no_duplicate_invite_codes() {
        const N: usize = 2000;
        let s = OsEntropySource;
        let mut seen = std::collections::HashSet::with_capacity(N);
        for i in 0..N {
            let c = new_invite_code(&s);
            assert_eq!(c.len(), INVITE_CODE_HEX_LEN, "第 {i} 次邀请码长度不对");
            assert!(
                seen.insert(c.clone()),
                "第 {i} 次邀请码与之前某次完全相同：{c}"
            );
        }
        assert_eq!(seen.len(), N);
    }

    /// 真 CSPRNG 与伪源**产出不同** —— 证明真的换了实现，而不是绕过。
    ///
    /// ⚠️ 这条断言的判据是「两个熵源在**同一进程内**取 2000 次，两边各自的
    /// 输出集合没有交集」。伪源与真源的分布若相同，概率同样是 2×10⁻⁷⁰。
    /// 只比一次没有说服力（比一次是 2⁻¹²⁸ 的运气）。
    #[test]
    fn os_and_non_csprng_sources_produce_disjoint_outputs() {
        const N: usize = 2000;
        let real = OsEntropySource;
        let fake = NonCsprngEntropySource::new();
        let real_set: std::collections::HashSet<String> =
            (0..N).map(|_| new_session_token(&real)).collect();
        let fake_set: std::collections::HashSet<String> =
            (0..N).map(|_| new_session_token(&fake)).collect();
        assert_eq!(real_set.len(), N, "真源自身先应无重复");
        assert_eq!(fake_set.len(), N, "伪源自身也应无重复（装置可信性）");
        let overlap: Vec<&String> = real_set.intersection(&fake_set).collect();
        assert!(
            overlap.is_empty(),
            "两个熵源产出了相同的令牌 {:?} —— 说明 `OsEntropySource` 根本没换实现",
            overlap
        );
    }

    /// P0 缺口的**精确机制**，且是这条测试写对了才留得下来的版本。
    ///
    /// 初版这里断言「两个实例共享同一份种子」—— **实测是错的**：
    /// [`os_seed`] 每次都新建 `RandomState`，而 `RandomState::new()` 会推进
    /// 线程局部的 k0，所以两次调用拿到的是**不同**的值。
    /// 正确的机制是：这些值全部由**同一个进程级 128 位种子**派生，
    /// 且每次取值只是把一个可预测的计数器拼进去 ——
    /// 也就是说，整个进程生命周期内的令牌熵**上限就是那 128 位**，
    /// 而不是每个令牌各 256 位。
    #[test]
    fn the_fake_source_derives_everything_from_one_process_level_secret() {
        // 每次 new() 拿到不同的 seed —— 但这**不是**独立的 OS 熵：
        // 它是同一个进程级 128 位密钥 + 一个单调推进的 k0 派生出来的。
        let a = NonCsprngEntropySource::new();
        let b = NonCsprngEntropySource::new();
        assert_ne!(
            a.seed, b.seed,
            "两次派生应当不同（RandomState 推进 k0）—— 若将来实现变了，这条要重新评估"
        );

        // 装置可信性：seed 不该是全零，否则上一条 `assert_ne!` 恒成立
        //（恒真断言 = 假闸门，是 AGENTS.md 明确禁止的那一类）。
        assert_ne!(os_seed(), [0u64, 0u64], "派生种子不应为全零");

        // 攻击面的大小就是这两块状态：16 字节种子 + 8 字节计数器。
        assert_eq!(
            std::mem::size_of_val(&a),
            24,
            "伪源的全部状态只有 24 字节：它的熵上限就是那 16 字节进程级种子"
        );

        // 反向对照：真源**零状态**，没有可被观测、可被复用的进程级秘密。
        assert_eq!(
            std::mem::size_of::<OsEntropySource>(),
            0,
            "真熵源必须零状态：它不能持有任何可被推导的进程级种子"
        );
    }

    /// 熵源失败路径被钉死：**panic**，且消息说清「不是配置问题」。
    ///
    /// 装置可信性：先证明成功路径不 panic，否则「panic 了」可能只是「总是 panic」。
    #[test]
    fn os_entropy_source_panics_when_the_os_entropy_source_fails() {
        let s = OsEntropySource;
        // 成功路径：真机上 getrandom 可用时不得 panic
        let mut ok = [0u8; TOKEN_BYTES];
        s.fill(&mut ok);
        assert!(
            ok.iter().any(|b| *b != 0),
            "真熵源不应产出全零（若全零说明它没在读内核熵池）"
        );

        // 失败路径：注入必然失败的读取器
        let mut out = [0u8; TOKEN_BYTES];
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fill_or_panic(&mut out, |_| Err(getrandom::Error::UNSUPPORTED));
        }));
        let payload = err.expect_err("熵源失败必须 panic，绝不能静默回退到弱随机");
        let msg = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .expect("panic 消息应是 String");
        assert!(
            msg.contains("系统熵源不可用"),
            "消息必须说清是熵源不可用，实际：{msg}"
        );
        assert!(
            msg.contains("不是配置问题"),
            "消息必须区分「不是配置问题」，否则值班会去翻环境变量。实际：{msg}"
        );
        assert!(
            msg.contains("getrandom"),
            "消息应指出失败的系统调用。实际：{msg}"
        );
    }

    /// 空切片不向内核要熵（否则零长度系统调用是无意义的失败）。
    /// 边界形态：非空要真熵、空切片不 panic，两条一起钉住。
    #[test]
    fn os_entropy_source_handles_the_empty_slice_boundary() {
        let s = OsEntropySource;
        let mut empty: [u8; 0] = [];
        s.fill(&mut empty);
        assert_eq!(random_bytes(&s, 0).len(), 0, "长度 0 必须仍然返回空 vec");
    }
}
