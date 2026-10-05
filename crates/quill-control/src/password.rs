use sha2::{Digest, Sha256};

use crate::error::ControlError;
use crate::secret::SecretSource;

pub const MIN_PASSWORD_LEN: usize = 12;

pub const MAX_PASSWORD_LEN: usize = 256;

pub const SALT_BYTES: usize = 16;

pub const DK_BYTES: usize = 32;

pub const PBKDF2_ALGO_PREFIX: &str = "pbkdf2-hmac-sha256";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pbkdf2Params {
    pub iterations: u32,
}

impl Pbkdf2Params {
    pub const fn production() -> Self {
        Self {
            iterations: 600_000,
        }
    }

    pub const MIN_ITERATIONS: u32 = 10_000;

    pub const fn for_tests() -> Self {
        Self {
            iterations: Self::MIN_ITERATIONS,
        }
    }

    pub fn algo_tag(&self) -> String {
        format!("{PBKDF2_ALGO_PREFIX}$i={}", self.iterations)
    }

    pub fn parse_algo(tag: &str) -> Result<Self, ControlError> {
        let rest =
            tag.strip_prefix(PBKDF2_ALGO_PREFIX)
                .ok_or_else(|| ControlError::InvariantBroken {
                    detail: format!(
                        "password_algo={tag:?} 不是本实现支持的 {PBKDF2_ALGO_PREFIX}$i=<次数>；\
                     该行的密码由另一个算法写入，本 crate 无从验证（需先补对应算法的验证分支）"
                    ),
                })?;
        let n = rest
            .strip_prefix("$i=")
            .ok_or_else(|| ControlError::InvariantBroken {
                detail: format!("password_algo={tag:?} 缺少 $i=<迭代次数> 段"),
            })?;
        let iterations: u32 = n.parse().map_err(|_| ControlError::InvariantBroken {
            detail: format!("password_algo={tag:?} 的迭代次数 {n:?} 不是数字"),
        })?;
        if iterations < Self::MIN_ITERATIONS {
            return Err(ControlError::InvariantBroken {
                detail: format!(
                    "password_algo={tag:?} 的迭代次数 {iterations} 低于下限 {}，\
                     该行密码等同明文存储，必须改密",
                    Self::MIN_ITERATIONS
                ),
            });
        }
        Ok(Self { iterations })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PasswordDigest {
    pub algo_tag: String,

    pub salt: [u8; SALT_BYTES],

    pub hash: [u8; DK_BYTES],
}

impl std::fmt::Debug for PasswordDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordDigest")
            .field("algo_tag", &self.algo_tag)
            .field("salt_len", &self.salt.len())
            .field("hash_len", &self.hash.len())
            .finish_non_exhaustive()
    }
}

pub fn validate_password(password: &str, username_norm: &str) -> Result<(), ControlError> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(ControlError::PasswordTooShort {
            len,
            min: MIN_PASSWORD_LEN,
        });
    }
    if len > MAX_PASSWORD_LEN {
        return Err(ControlError::PasswordTooLong {
            len,
            max: MAX_PASSWORD_LEN,
        });
    }
    if !username_norm.is_empty() && password.eq_ignore_ascii_case(username_norm) {
        return Err(ControlError::PasswordEqualsUsername);
    }
    Ok(())
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let h = Sha256::digest(key);
        k[..DK_BYTES].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
    assert!(iterations > 0, "PBKDF2 迭代次数必须 ≥ 1");
    assert!(dk_len > 0, "PBKDF2 输出长度必须 ≥ 1");

    let mut out = Vec::with_capacity(dk_len);
    let mut block_index: u32 = 1;
    while out.len() < dk_len {
        let mut first = Vec::with_capacity(salt.len() + 4);
        first.extend_from_slice(salt);
        first.extend_from_slice(&block_index.to_be_bytes());
        let mut u = hmac_sha256(password, &first);
        let mut t = u;
        for _ in 1..iterations {
            u = hmac_sha256(password, &u);
            for (acc, byte) in t.iter_mut().zip(u.iter()) {
                *acc ^= *byte;
            }
        }
        let take = (dk_len - out.len()).min(DK_BYTES);
        out.extend_from_slice(&t[..take]);
        block_index += 1;
    }
    out
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[derive(Debug)]
pub struct PasswordHasher {
    params: Pbkdf2Params,
}

impl PasswordHasher {
    pub fn new(params: Pbkdf2Params) -> Self {
        Self { params }
    }

    pub fn algo_tag(&self) -> String {
        self.params.algo_tag()
    }

    pub fn hash(&self, entropy: &dyn SecretSource, password: &str) -> PasswordDigest {
        let salt = crate::secret::random_bytes(entropy, SALT_BYTES);
        let mut salt_arr = [0u8; SALT_BYTES];
        salt_arr.copy_from_slice(&salt);
        let hash = pbkdf2_sha256(
            password.as_bytes(),
            &salt_arr,
            self.params.iterations,
            DK_BYTES,
        );
        let mut hash_arr = [0u8; DK_BYTES];
        hash_arr.copy_from_slice(&hash);
        PasswordDigest {
            algo_tag: self.params.algo_tag(),
            salt: salt_arr,
            hash: hash_arr,
        }
    }

    pub fn verify_stored(&self, stored: &PasswordDigest, password: &str) -> bool {
        let params = match Pbkdf2Params::parse_algo(&stored.algo_tag) {
            Ok(p) => p,

            Err(_) => return false,
        };
        let candidate = pbkdf2_sha256(
            password.as_bytes(),
            &stored.salt,
            params.iterations,
            DK_BYTES,
        );
        constant_time_eq(&candidate, &stored.hash)
    }

    pub fn burn_equivalent_time(&self) {
        let _ = pbkdf2_sha256(
            b"quill-timing-equalizer",
            &[0u8; SALT_BYTES],
            self.params.iterations,
            DK_BYTES,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{hex_decode_exact, hex_encode, NonCsprngEntropySource};

    #[derive(Debug, Default)]
    struct FixedSalt;

    impl SecretSource for FixedSalt {
        fn fill(&self, out: &mut [u8]) {
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = u8::try_from(i).unwrap_or(0);
            }
        }
    }

    #[test]
    fn hmac_matches_published_vectors() {
        let cases: [(&[u8], &[u8], &str); 3] = [
            (
                &[0x0b; 20],
                b"Hi There",
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe",
                b"what do ya want for nothing?",
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                &[0xaa; 20],
                &[0xdd; 50],
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
        ];
        for (key, msg, want) in cases {
            assert_eq!(
                hex_encode(&hmac_sha256(key, msg)),
                want,
                "HMAC-SHA256(key={key:?}, msg={msg:?}) 与 RFC 向量不符"
            );
        }
    }

    #[test]
    fn hmac_handles_keys_longer_than_the_block_size() {
        let key = [0xaau8; 131];
        assert_eq!(key.len(), 131);
        let got = hex_encode(&hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        ));
        assert_eq!(
            got, "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            "超长 key 的 HMAC-SHA256 与 RFC 4231 用例 6 不符"
        );
    }

    struct Pbkdf2Vector {
        pw: &'static [u8],
        salt: &'static [u8],
        iter: u32,
        dk: usize,
        want: &'static str,
    }

    #[test]
    fn pbkdf2_matches_published_vectors() {
        let cases = [
            Pbkdf2Vector {
                pw: b"password",
                salt: b"salt",
                iter: 1,
                dk: 32,
                want: "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",
            },
            Pbkdf2Vector {
                pw: b"password",
                salt: b"salt",
                iter: 2,
                dk: 32,
                want: "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43",
            },
            Pbkdf2Vector {
                pw: b"password",
                salt: b"salt",
                iter: 4096,
                dk: 32,
                want: "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a",
            },
            Pbkdf2Vector {
                pw: b"passwordPASSWORDpassword",
                salt: b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
                iter: 4096,
                dk: 32,
                want: "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1",
            },
            Pbkdf2Vector {
                pw: b"pass\0word",
                salt: b"sa\0lt",
                iter: 4096,
                dk: 32,
                want: "89b69d0516f829893c696226650a86878c029ac13ee276509d5ae58b6466a724",
            },
        ];
        for Pbkdf2Vector {
            pw,
            salt,
            iter,
            dk,
            want,
        } in cases
        {
            let got = hex_encode(&pbkdf2_sha256(pw, salt, iter, dk));
            assert_eq!(
                got, want,
                "PBKDF2(pw={pw:?}, salt={salt:?}, i={iter}) 与参考不符"
            );
        }
    }

    #[test]
    fn pbkdf2_multi_block_matches_reference() {
        let short = pbkdf2_sha256(b"password", b"salt", 2, 40);
        let long = pbkdf2_sha256(b"password", b"salt", 2, 100);
        assert_eq!(short.len(), 40);
        assert_eq!(long.len(), 100);
        assert_eq!(
            &long[..40],
            &short[..],
            "PBKDF2 分块依赖 dklen —— 块边界算错了"
        );
        assert_eq!(
            hex_encode(&short),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43830651afcb5c862f",
            "dklen=40 的分块结果与参考不符"
        );
    }

    #[test]
    fn constant_time_eq_is_correct_on_equal_unequal_and_different_lengths() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(&[1, 2, 3], &[1, 2, 3]));
        assert!(!constant_time_eq(&[1, 2, 3], &[1, 2, 4]));
        assert!(!constant_time_eq(&[1, 2, 3], &[1, 2]));
        assert!(!constant_time_eq(b"a", b"A"), "大小写不同必须判不等");

        assert!(!constant_time_eq(&[0u8; 32], &[1u8; 32]));
    }

    #[test]
    fn algo_tag_roundtrip() {
        for iters in [10_000u32, 100_000, 600_000, 4_294_967_295] {
            let p = Pbkdf2Params { iterations: iters };
            let tag = p.algo_tag();
            assert_eq!(tag, format!("{PBKDF2_ALGO_PREFIX}$i={iters}"));
            assert_eq!(Pbkdf2Params::parse_algo(&tag).unwrap(), p);
        }
    }

    #[test]
    fn parse_algo_rejects_weakened_or_foreign_tags_loudly() {
        for bad in [
            "argon2id",
            "",
            "pbkdf2-hmac-sha256",
            "pbkdf2-hmac-sha256$i=",
            "pbkdf2-hmac-sha256$i=abc",
            "pbkdf2-hmac-sha256$i=0",
            "pbkdf2-hmac-sha256$i=1",
            "pbkdf2-hmac-sha256$i=9999",
            "pbkdf2-hmac-sha256$I=600000",
        ] {
            let r = Pbkdf2Params::parse_algo(bad);
            assert!(
                r.is_err(),
                "标签 {bad:?} 必须被判失败（否则会默默用弱参数验证）"
            );
        }

        assert!(Pbkdf2Params::parse_algo("pbkdf2-hmac-sha256$i=10000").is_ok());
    }

    #[test]
    fn production_params_meet_the_documented_floor() {
        assert!(
            Pbkdf2Params::production().iterations >= 600_000,
            "生产迭代次数低于 OWASP 对 PBKDF2-HMAC-SHA256 的推荐值"
        );
    }

    #[test]
    fn hash_then_verify_accepts_right_password_and_rejects_wrong() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        let salt = FixedSalt;
        let d = hasher.hash(&salt, "correct-horse-battery");

        assert!(
            hasher.verify_stored(&d, "correct-horse-battery"),
            "正确密码应通过"
        );
        assert!(
            !hasher.verify_stored(&d, "correct-horse-batterY"),
            "大小写不同应被拒"
        );
        assert!(
            !hasher.verify_stored(&d, "correct-horse-battery "),
            "多一个空格应被拒"
        );
        assert!(!hasher.verify_stored(&d, ""), "空密码应被拒");
    }

    #[test]
    fn same_password_twice_yields_different_digest() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        let e = NonCsprngEntropySource::new();
        let a = hasher.hash(&e, "correct-horse-battery");
        let b = hasher.hash(&e, "correct-horse-battery");
        assert_ne!(a.hash, b.hash, "两次哈希结果相同 → 盐没起作用");
        assert_ne!(a.salt, b.salt, "两次盐相同 → 随机源没起作用");
        assert_eq!(a.algo_tag, b.algo_tag);

        assert!(hasher.verify_stored(&a, "correct-horse-battery"));
        assert!(hasher.verify_stored(&b, "correct-horse-battery"));
    }

    #[test]
    fn verify_refuses_rows_whose_algo_tag_is_not_understood() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        let stored = PasswordDigest {
            algo_tag: "argon2id".to_string(),
            salt: [0u8; SALT_BYTES],
            hash: [0u8; DK_BYTES],
        };
        assert!(!hasher.verify_stored(&stored, "whatever-password-x"));
    }

    #[test]
    fn verify_uses_the_params_stored_in_the_row_not_current_config() {
        let weak = PasswordHasher::new(Pbkdf2Params::for_tests());
        let salt = FixedSalt;
        let legacy = weak.hash(&salt, "correct-horse-battery");
        assert!(legacy.algo_tag.ends_with("$i=10000"));

        let strong = PasswordHasher::new(Pbkdf2Params::production());
        assert!(
            strong.verify_stored(&legacy, "correct-horse-battery"),
            "改配置后老用户被登出 = 升级事故"
        );
        assert!(!strong.verify_stored(&legacy, "wrong-password-x"));
    }

    #[test]
    fn digest_debug_does_not_leak_material() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        let d = hasher.hash(&FixedSalt, "correct-horse-battery");
        let salt_hex = hex_encode(&d.salt);
        let hash_hex = hex_encode(&d.hash);
        let shown = format!("{d:?}");
        assert!(!shown.contains(&salt_hex), "Debug 泄露了盐：{shown}");
        assert!(!shown.contains(&hash_hex), "Debug 泄露了哈希：{shown}");
        assert!(
            !shown.contains("correct-horse-battery"),
            "Debug 泄露了密码原文：{shown}"
        );

        assert_eq!(salt_hex.len(), SALT_BYTES * 2);
        assert_eq!(
            hex_decode_exact::<DK_BYTES>(&hash_hex, || ControlError::TokenMalformed)
                .unwrap()
                .len(),
            DK_BYTES
        );

        assert!(
            shown.contains(PBKDF2_ALGO_PREFIX),
            "Debug 应显示算法：{shown}"
        );
        assert!(shown.contains("salt_len"), "Debug 应显示盐长度：{shown}");
    }

    #[test]
    fn password_policy_bounds_and_username_collision() {
        assert!(validate_password("correct-horse-battery", "zhang_wei").is_ok());
        assert_eq!(
            validate_password("short", "zhang_wei"),
            Err(ControlError::PasswordTooShort {
                len: 5,
                min: MIN_PASSWORD_LEN
            })
        );

        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN), "u").is_ok());
        assert_eq!(
            validate_password(&"a".repeat(MIN_PASSWORD_LEN - 1), "u"),
            Err(ControlError::PasswordTooShort {
                len: MIN_PASSWORD_LEN - 1,
                min: MIN_PASSWORD_LEN
            })
        );

        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LEN), "u").is_ok());
        assert!(matches!(
            validate_password(&"a".repeat(MAX_PASSWORD_LEN + 1), "u"),
            Err(ControlError::PasswordTooLong { .. })
        ));

        assert!(validate_password(&"中".repeat(12), "u").is_ok());

        assert_eq!(
            validate_password("ZhangWei2024", "zhangwei2024"),
            Err(ControlError::PasswordEqualsUsername),
            "密码与用户名（忽略大小写）相同必须被拒"
        );

        assert!(validate_password("zhangwei2024-extra", "zhangwei2024").is_ok());
    }

    #[test]
    fn burn_equivalent_time_produces_no_output_and_does_not_panic() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        hasher.burn_equivalent_time();

        let t0 = std::time::Instant::now();
        hasher.burn_equivalent_time();
        let spent = t0.elapsed();
        assert!(
            spent.as_millis() >= 1,
            "计时均衡器没消耗时间（{spent:?}）—— 用户名枚举防线失效"
        );
    }
}
