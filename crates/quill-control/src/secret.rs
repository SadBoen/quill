#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::error::ControlError;

pub const TOKEN_BYTES: usize = 32;

pub const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

pub const INVITE_CODE_BYTES: usize = 16;

pub const INVITE_CODE_HEX_LEN: usize = INVITE_CODE_BYTES * 2;

pub trait SecretSource: Send + Sync + 'static {
    fn fill(&self, out: &mut [u8]);
}

#[derive(Debug, Default)]
pub struct OsEntropySource;

pub fn entropy_unavailable_message(err: &getrandom::Error) -> String {
    format!(
        "系统熵源不可用：getrandom 失败（{err}）。\
         这不是配置问题，排查环境变量没有意义——内核熵池取不到随机数时没有安全的替代值。\
         请检查宿主机内核版本 / 容器是否限制了 getrandom(2) / seccomp 是否拦截该系统调用。\
         进程终止是预期行为：继续运行只会签发可被推算的会话令牌。"
    )
}

fn fill_or_panic(out: &mut [u8], read: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>) {
    if let Err(e) = read(out) {
        panic!("{}", entropy_unavailable_message(&e));
    }
}

impl SecretSource for OsEntropySource {
    fn fill(&self, out: &mut [u8]) {
        if out.is_empty() {
            return;
        }
        fill_or_panic(out, getrandom::fill);
    }
}

#[cfg(test)]
#[derive(Debug)]
pub struct NonCsprngEntropySource {
    counter: AtomicU64,
    seed: [u64; 2],
}

#[cfg(test)]
impl NonCsprngEntropySource {
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

#[cfg(test)]
fn os_seed() -> [u64; 2] {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

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

pub fn random_bytes(src: &dyn SecretSource, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    src.fill(&mut out);
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// 小写 hex。实现已收口到 `quill_adapters::ids::to_hex_lower`（全项目唯一出口）。
///
/// **解码方向刻意不收口**：下面的 `hex_val` 只认小写，`hex_decode_exact` 因此
/// 拒绝大写 token —— 那是安全语义（token 必须是小写形态）。`ids::hex_val`
/// 两种大小写都收，换过去会把「大写 token 也接受」悄悄放进来。
pub fn hex_encode(bytes: &[u8]) -> String {
    quill_adapters::ids::to_hex_lower(bytes)
}

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

pub fn new_session_token(src: &dyn SecretSource) -> String {
    hex_encode(&random_bytes(src, TOKEN_BYTES))
}

pub fn session_token_digest(token: &str) -> Result<[u8; 32], ControlError> {
    let raw = hex_decode_exact::<{ TOKEN_BYTES }>(token, || ControlError::TokenMalformed)?;

    Ok(sha256(&raw))
}

pub fn new_invite_code(src: &dyn SecretSource) -> String {
    hex_encode(&random_bytes(src, INVITE_CODE_BYTES))
}

pub fn invite_code_digest(code: &str) -> Result<[u8; 32], ControlError> {
    let raw = hex_decode_exact::<{ INVITE_CODE_BYTES }>(code, || ControlError::InviteMalformed)?;
    Ok(sha256(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(
            hex_decode_exact::<32>(&"0".repeat(64), e).is_ok(),
            "合法输入应通过"
        );
        assert_eq!(hex_decode_exact::<32>(&"0".repeat(63), e), Err(e()));
        assert_eq!(hex_decode_exact::<32>(&"0".repeat(65), e), Err(e()));
        assert_eq!(hex_decode_exact::<32>("", e), Err(e()));

        assert_eq!(hex_decode_exact::<32>(&"A".repeat(64), e), Err(e()));
        let with_bad_tail = "0".repeat(63) + "g";
        assert_eq!(hex_decode_exact::<32>(&with_bad_tail, e), Err(e()));
    }

    #[test]
    fn hex_decode_exact_handles_other_widths() {
        let e = || ControlError::InviteMalformed;
        let c = "0123456789abcdef0123456789abcdef";
        assert_eq!(c.len(), INVITE_CODE_HEX_LEN);
        let b = hex_decode_exact::<{ INVITE_CODE_BYTES }>(c, e).expect("合法邀请码应能解析");
        assert_eq!(b[0], 0x01);
        assert_eq!(b[15], 0xef);

        assert_eq!(
            hex_decode_exact::<{ INVITE_CODE_BYTES }>(&"0".repeat(33), e),
            Err(e())
        );
    }

    #[test]
    fn sha256_matches_published_vector() {
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

        let other = "0".repeat(63) + "1";
        assert_ne!(d, session_token_digest(&other).expect("格式合法"));

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

        let c = random_bytes(&NonCsprngEntropySource::new(), TOKEN_BYTES);
        assert_ne!(a, c);
    }

    #[test]
    fn zero_source_still_produces_full_length_output() {
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

        assert_ne!(t, new_session_token(&s));
    }

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

    #[test]
    fn the_fake_source_derives_everything_from_one_process_level_secret() {
        let a = NonCsprngEntropySource::new();
        let b = NonCsprngEntropySource::new();
        assert_ne!(
            a.seed, b.seed,
            "两次派生应当不同（RandomState 推进 k0）—— 若将来实现变了，这条要重新评估"
        );

        assert_ne!(os_seed(), [0u64, 0u64], "派生种子不应为全零");

        assert_eq!(
            std::mem::size_of_val(&a),
            24,
            "伪源的全部状态只有 24 字节：它的熵上限就是那 16 字节进程级种子"
        );

        assert_eq!(
            std::mem::size_of::<OsEntropySource>(),
            0,
            "真熵源必须零状态：它不能持有任何可被推导的进程级种子"
        );
    }

    #[test]
    fn os_entropy_source_panics_when_the_os_entropy_source_fails() {
        let s = OsEntropySource;

        let mut ok = [0u8; TOKEN_BYTES];
        s.fill(&mut ok);
        assert!(
            ok.iter().any(|b| *b != 0),
            "真熵源不应产出全零（若全零说明它没在读内核熵池）"
        );

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

    #[test]
    fn os_entropy_source_handles_the_empty_slice_boundary() {
        let s = OsEntropySource;
        let mut empty: [u8; 0] = [];
        s.fill(&mut empty);
        assert_eq!(random_bytes(&s, 0).len(), 0, "长度 0 必须仍然返回空 vec");
    }
}
