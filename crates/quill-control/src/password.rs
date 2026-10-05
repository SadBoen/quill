//! 密码哈希：PBKDF2-HMAC-SHA256（RFC 2898 / RFC 8018）。
//!
//! # 🔴 已知缺口：schema 期望的是 argon2id，本实现是 PBKDF2 —— **临时方案**
//!
//! `crates/quill-store/migrations/0001_init.sql` 里
//! `users.password_algo TEXT NOT NULL DEFAULT 'argon2id'` —— schema 的**默认**算法是 argon2id。
//! 但：
//!
//! - `Cargo.lock` 里**没有** `argon2`（也没有 `scrypt`、`password-hash`、`rand`）；
//! - 本任务被硬性要求**不新增任何外部 crate 依赖**。
//!
//! 而 `AGENTS.md` 的要求是「不要自己发明」——**自创一个密码哈希方案是不可接受的**
//! （自创 = 无人审计 = 迟早被暴力枚举攻破）。所以本模块的选法是：
//!
//! | 方案 | 判断 |
//! |---|---|
//! | 自创哈希 | ❌ 违反「不要自己发明」 |
//! | 明文 / 单轮 SHA-256 存密码 | ❌ 直接可被彩虹表与 GPU 秒破 |
//! | 纯随机令牌当密码 | ❌ 用户输错就永久锁死，无法自助改密 |
//! | **PBKDF2-HMAC-SHA256（标准，有 RFC、有公开测试向量）** | ✅ **最简可用**，缺口上报主理人 |
//!
//! # 上线前必须换掉
//!
//! 换成 `argon2` crate（`Argon2id`，m=19MiB/t=2/p=1 是 OWASP 现行推荐），
//! **且必须保留旧算法的验证路径**：把 `password_algo` 的取值当作调度器，
//! 本模块的 [`Pbkdf2Params::parse_algo`] 已经是这个形状 ——
//! 它按字符串分派，所以加 argon2 分支是**加一个分支**而不是重写。
//!
//! # 为什么参数存在 `password_algo` 字符串里而不是另开一列
//!
//! `users` 表**没有** iterations 列（不可改根 `Cargo.toml` 之外的 schema，
//! 且铁律六要求迁移 expand-only）。把参数编进 `password_algo` 的好处是：
//!
//! - 参数与算法**同行自描述** → 换算法/提参数时老行仍可验证（登录时按老参数验）；
//! - 不需要改 schema，不需要迁移；
//! - 缺参数会被 [`Pbkdf2Params::parse_algo`] **判失败**而不是默默用默认值
//!   （默默用默认值 = 有人把迭代数改成 1，你永远发现不了）。
//!
//! # 正确性凭据：不靠自建 fixture，靠**外部权威向量**
//!
//! `tests::hmac_matches_published_vectors` / `tests::pbkdf2_matches_published_vectors`
//! 用的是 **CPython `hashlib`（OpenSSL 后端）**产出的向量，
//! 复现命令写在测试注释里。铁律十五要求「修闸门用外部独立 fixture 复验」，
//! 自己造一组期望值再让自己通过，等于自己和自己对答案。

use sha2::{Digest, Sha256};

use crate::error::ControlError;
use crate::secret::SecretSource;

/// 密码最短长度（字符数）。
///
/// 取 12：NIST SP 800-63B 建议 ≥ 8 且**不限组成规则**，
/// 本项目取更严的 12 是因为密码一旦泄露要顶住离线爆破。
pub const MIN_PASSWORD_LEN: usize = 12;

/// 密码最长长度（字符数）。
///
/// **必须有上限**：PBKDF2 的成本与密码长度成正比，
/// 不封顶就等于给了一个「用 10 MB 密码把 CPU 打满」的 DoS 入口。
pub const MAX_PASSWORD_LEN: usize = 256;

/// 盐字节数。
///
/// 取 16：NIST SP 800-132 要求 ≥ 16 字节。
pub const SALT_BYTES: usize = 16;

/// 派生密钥字节数（= SHA-256 输出长度）。
pub const DK_BYTES: usize = 32;

/// `password_algo` 的算法前缀。
pub const PBKDF2_ALGO_PREFIX: &str = "pbkdf2-hmac-sha256";

/// PBKDF2 迭代次数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pbkdf2Params {
    /// 迭代次数（必须是 ≥ 1；生产值见 [`Pbkdf2Params::production`]）。
    pub iterations: u32,
}

impl Pbkdf2Params {
    /// 生产默认值：600 000 次。
    ///
    /// 依据：OWASP Password Storage Cheat Sheet 对 PBKDF2-HMAC-SHA256 的推荐值。
    /// ⚠️ 换到 argon2id 后本常量应随之下线（见模块文档）。
    pub const fn production() -> Self {
        Self {
            iterations: 600_000,
        }
    }

    /// 迭代次数下限：低于此值一律拒绝。
    ///
    /// 存在的理由：有人手滑把 `i=1` 写进 `password_algo` 时，
    /// 登录会**变快 60 万倍**且没有任何报错。这里让它变成响亮的失败。
    pub const MIN_ITERATIONS: u32 = 10_000;

    /// 测试用低迭代次数。
    ///
    /// ⚠️ **只允许测试用**：它让 PBKDF2 便宜到能在 `cargo test`（debug，未优化）里跑。
    /// 生产路径必须用 [`Pbkdf2Params::production`] —— 由
    /// `tests::control_plane.rs::production_params_are_not_weakened` 守住。
    pub const fn for_tests() -> Self {
        Self {
            iterations: Self::MIN_ITERATIONS,
        }
    }

    /// 编成 `password_algo` 列的取值：`pbkdf2-hmac-sha256$i=600000`。
    pub fn algo_tag(&self) -> String {
        format!("{PBKDF2_ALGO_PREFIX}$i={}", self.iterations)
    }

    /// 从 `password_algo` 解析回参数。
    ///
    /// 严格：前缀不认识、格式不对、迭代数低于下限 → 全部 [`ControlError::InvariantBroken`]。
    ///
    /// 为什么用 `InvariantBroken` 而不是 `CredentialsRejected`：
    /// 行里的算法标记是**我们自己写进去的**，它不对意味着数据被外部改坏或代码有 bug，
    /// 说成「密码错」会把一个真故障伪装成一次普通登录失败。
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

/// 一条密码摘要（对应 `users` 表的 `password_hash` / `password_salt` / `password_algo`）。
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordDigest {
    /// `password_algo` 取值。
    pub algo_tag: String,
    /// `password_salt`（16 字节）。
    pub salt: [u8; SALT_BYTES],
    /// `password_hash`（32 字节）。
    pub hash: [u8; DK_BYTES],
}

impl std::fmt::Debug for PasswordDigest {
    /// ⚠️ **绝不打哈希与盐的明文**。
    ///
    /// 理由：`Debug` 会进日志。`users.password_hash` 落进日志等于
    /// 把「离线爆破用的完整材料」抄送了一份。
    /// 因此这里只打算法与长度 —— 足够定位「用的哪套参数」，又不泄密。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordDigest")
            .field("algo_tag", &self.algo_tag)
            .field("salt_len", &self.salt.len())
            .field("hash_len", &self.hash.len())
            .finish_non_exhaustive()
    }
}

/// 密码策略校验。
///
/// 规则（刻意只有长度与「不等于用户名」两条）：
/// 组成复杂度规则（必须含大小写数字符号）在 NIST SP 800-63B 里已被**明确劝退** ——
/// 它把用户推向 `Password1!` 这种可预测模式，实际反而更弱。
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

/// HMAC-SHA256。
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    // 手工实现 HMAC（RFC 2104）：K ⊕ opad / K ⊕ ipad。
    // ⚠️ key 比块长（64 字节）长时必须先哈希 —— 少了这一步，
    //    长密钥的 HMAC 就是错的（且只在「密钥很长」时才错，最难发现）。
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

/// PBKDF2-HMAC-SHA256（RFC 8018 §5.2）。
///
/// `dk_len` 大于 32 时按 `T_1 / T_2 / …` 分块迭代 —— 这一段是 PBKDF2 最常写错的地方，
/// 所以 `tests::pbkdf2_multi_block_matches_reference` 专门用外部向量钉住它。
///
/// # Panics
///
/// `iterations == 0` 时 panic：那不是「弱密码」而是**无意义输入**
/// （PBKDF2 定义要求 T ≥ 1，产出恒为全零哈希）。
/// `dk_len == 0` 同理。调用方（[`Pbkdf2Params::parse_algo`]）已挡住 `iterations < 10000`。
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
    assert!(iterations > 0, "PBKDF2 迭代次数必须 ≥ 1");
    assert!(dk_len > 0, "PBKDF2 输出长度必须 ≥ 1");

    let mut out = Vec::with_capacity(dk_len);
    let mut block_index: u32 = 1;
    while out.len() < dk_len {
        // U_1 = PRF(P, S || INT_32_BE(i)) = HMAC-SHA256(password, salt || i)
        let mut first = Vec::with_capacity(salt.len() + 4);
        first.extend_from_slice(salt);
        first.extend_from_slice(&block_index.to_be_bytes());
        let mut u = hmac_sha256(password, &first);
        let mut t = u;
        for _ in 1..iterations {
            // ⚠️ U_{c} = PRF(P, U_{c-1}) —— PRF **就是 HMAC**，
            //    不是裸哈希。写成 sha256(&u) 时 i=1 的向量照样通过
            //    （循环体不执行），只有 i>=2 才暴露 —— 这就是
            //    「自建 fixture 会与实现一起错」的典型：必须用外部向量。
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

/// 定长常数时间比较。
///
/// **必须**用它而不是 `==`：`==` 一旦发现首个不等字节就短路返回，
/// 于是比较耗时随「前 n 字节是否相同」变化 —— 攻击者靠计时能逐字节试出摘要。
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

/// 密码哈希器。
#[derive(Debug)]
pub struct PasswordHasher {
    params: Pbkdf2Params,
}

impl PasswordHasher {
    /// 构造。
    pub fn new(params: Pbkdf2Params) -> Self {
        Self { params }
    }

    /// 当前的 `password_algo` 取值（建行时直接写它）。
    pub fn algo_tag(&self) -> String {
        self.params.algo_tag()
    }

    /// 生成随机盐并派生摘要。
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

    /// 按**摘要自带**的算法标签验证（登录路径必须走这里）。
    ///
    /// ⚠️ 参数取自数据库行而不是本 `PasswordHasher` 的配置 ——
    /// 这样将来「默认迭代数提高」之后，老用户仍按老参数验证成功，
    /// 不会因为改配置而集体登出（那是升级事故的经典形态）。
    pub fn verify_stored(&self, stored: &PasswordDigest, password: &str) -> bool {
        let params = match Pbkdf2Params::parse_algo(&stored.algo_tag) {
            Ok(p) => p,
            // 参数无法解析 = 该行不可验证。返回 false（拒绝登录）而不是 true。
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

    /// 消耗一次与真实登录等价的 CPU 时间。
    ///
    /// 用途：`login()` 在「用户名不存在」时也调用它 ——
    /// 否则「用户不存在」比「密码错」快几十毫秒，**登录接口就成了用户枚举器**
    /// （按响应时间二分即可列出哪些用户名存在）。
    /// 这不是洁癖：用户名枚举是接管账号的第一步。
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

    /// 确定性盐源：产出 `00 01 02 …`，让摘要断言可复现。
    #[derive(Debug, Default)]
    struct FixedSalt;

    impl SecretSource for FixedSalt {
        fn fill(&self, out: &mut [u8]) {
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = u8::try_from(i).unwrap_or(0);
            }
        }
    }

    // ── 外部权威向量 ────────────────────────────────────────────────────
    //
    // 复现命令（向量来源 = CPython hashlib，OpenSSL 后端，与本实现完全独立）：
    //   python3 -c 'import hashlib; print(hashlib.pbkdf2_hmac("sha256", b"password", b"salt", 4096, 32).hex())'

    #[test]
    fn hmac_matches_published_vectors() {
        // RFC 4231 HMAC-SHA256 测试用例 1/2/3
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
        // RFC 4231 用例 6：key = 0xaa 重复 131 次（> 64 字节块长）。
        // 这一条专门盯「key 超长时先哈希」那一步 —— 漏了它只有长密钥会错。
        // 期望值同样来自 RFC 4231。
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

    /// 一条 PBKDF2 参考向量。
    ///
    /// 用具名字段而不是 5 元组：元组里 `dk` 与 `iter` 都是数字，
    /// 写反了类型不变、结果只差在断言信息里。
    struct Pbkdf2Vector {
        pw: &'static [u8],
        salt: &'static [u8],
        iter: u32,
        dk: usize,
        want: &'static str,
    }

    #[test]
    fn pbkdf2_matches_published_vectors() {
        // 与 RFC 6070（HMAC-SHA1 版）同构的 PBKDF2-HMAC-SHA256 向量，
        // 由 CPython hashlib（OpenSSL）产出：见模块文档的复现命令。
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
        // 分块循环专项：dklen > 32 时各块 T_1/T_2/... 与 dklen 无关，只在末尾截断。
        // 所以「dklen=100 的前 40 字节」必须等于「dklen=40 的输出」—— 这条
        // 交叉性质比单个向量更能暴露「按 dklen 算块数」的错实现。
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
        // 全零 vs 全一：最容易写的「==」在这里必须判不等
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
        // 关键性质：这些情况必须**响亮失败**，绝不能默默用默认参数验证
        // （默默兜底 = 有人把 i 改成 1 你永远发现不了 = 密码等同明文）。
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
        // 装置可信性：合法标签确实能过
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
        // 装置可信性：合法数据先能验过，否则「拒绝错密码」可能只是在测空气
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
        // 盐必须真的起作用：两次相同密码的摘要必须不同，
        // 否则「同密码用户可互相彩虹表比对」这条防线就没了。
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        let e = NonCsprngEntropySource::new();
        let a = hasher.hash(&e, "correct-horse-battery");
        let b = hasher.hash(&e, "correct-horse-battery");
        assert_ne!(a.hash, b.hash, "两次哈希结果相同 → 盐没起作用");
        assert_ne!(a.salt, b.salt, "两次盐相同 → 随机源没起作用");
        assert_eq!(a.algo_tag, b.algo_tag);
        // 但两个摘要都必须能验证同一个密码
        assert!(hasher.verify_stored(&a, "correct-horse-battery"));
        assert!(hasher.verify_stored(&b, "correct-horse-battery"));
    }

    #[test]
    fn verify_refuses_rows_whose_algo_tag_is_not_understood() {
        // 模拟「行由 argon2 写入」：本 crate 无从验证 → 必须拒绝登录，
        // 而不是「当成密码对」或「panic」。
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
        // 升级场景：默认迭代数从 10_000 提到 600_000 后，
        // 老用户（10_000 写入的行）必须仍能登录。
        let weak = PasswordHasher::new(Pbkdf2Params::for_tests());
        let salt = FixedSalt;
        let legacy = weak.hash(&salt, "correct-horse-battery");
        assert!(legacy.algo_tag.ends_with("$i=10000"));

        // 换成生产参数的 hasher 再验同一行：仍必须通过
        let strong = PasswordHasher::new(Pbkdf2Params::production());
        assert!(
            strong.verify_stored(&legacy, "correct-horse-battery"),
            "改配置后老用户被登出 = 升级事故"
        );
        assert!(!strong.verify_stored(&legacy, "wrong-password-x"));
    }

    #[test]
    fn digest_debug_does_not_leak_material() {
        // 装置可信性：先证明摘要里真的有可爆破材料，再证明 Debug 没打它。
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
        // 反向断言：确实有材料可藏（否则上面恒真）
        assert_eq!(salt_hex.len(), SALT_BYTES * 2);
        assert_eq!(
            hex_decode_exact::<DK_BYTES>(&hash_hex, || ControlError::TokenMalformed)
                .unwrap()
                .len(),
            DK_BYTES
        );
        // 但算法与长度必须可见 —— 排障要靠它
        assert!(
            shown.contains(PBKDF2_ALGO_PREFIX),
            "Debug 应显示算法：{shown}"
        );
        assert!(shown.contains("salt_len"), "Debug 应显示盐长度：{shown}");
    }

    #[test]
    fn password_policy_bounds_and_username_collision() {
        // 装置可信性：合法密码先能过
        assert!(validate_password("correct-horse-battery", "zhang_wei").is_ok());
        assert_eq!(
            validate_password("short", "zhang_wei"),
            Err(ControlError::PasswordTooShort {
                len: 5,
                min: MIN_PASSWORD_LEN
            })
        );
        // 恰好等于下限 → 通过（边界不多算一位）
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN), "u").is_ok());
        assert_eq!(
            validate_password(&"a".repeat(MIN_PASSWORD_LEN - 1), "u"),
            Err(ControlError::PasswordTooShort {
                len: MIN_PASSWORD_LEN - 1,
                min: MIN_PASSWORD_LEN
            })
        );
        // 上限边界
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LEN), "u").is_ok());
        assert!(matches!(
            validate_password(&"a".repeat(MAX_PASSWORD_LEN + 1), "u"),
            Err(ControlError::PasswordTooLong { .. })
        ));
        // 长度按字符计，不按字节：一个 12 字符的中文密码应通过
        assert!(validate_password(&"中".repeat(12), "u").is_ok());
        // 密码 == 用户名（大小写不敏感）必须被拒。
        // ⚠️ 用户名取 12 字符是刻意的：密码策略先查长度（≥12），
        //    用户名短于 12 时会先被 TooShort 拦下，根本走不到这条规则 ——
        //    那样这条断言就只是在测「长度不足」，不是测「密码撞用户名」。
        assert_eq!(
            validate_password("ZhangWei2024", "zhangwei2024"),
            Err(ControlError::PasswordEqualsUsername),
            "密码与用户名（忽略大小写）相同必须被拒"
        );
        // 但「只是包含用户名」不算撞名（那是常见且可接受的口令形态）
        assert!(validate_password("zhangwei2024-extra", "zhangwei2024").is_ok());
    }

    #[test]
    fn burn_equivalent_time_produces_no_output_and_does_not_panic() {
        let hasher = PasswordHasher::new(Pbkdf2Params::for_tests());
        hasher.burn_equivalent_time();
        // 装置可信性：计时均衡器必须真的消耗时间，否则它只是「一个空函数」。
        let t0 = std::time::Instant::now();
        hasher.burn_equivalent_time();
        let spent = t0.elapsed();
        assert!(
            spent.as_millis() >= 1,
            "计时均衡器没消耗时间（{spent:?}）—— 用户名枚举防线失效"
        );
    }
}
