//! 内容摘要：SHA-256 + hex 编码。
//!
//! # 为什么自己写 hex 而不用 `hex` crate
//!
//! `hex` 虽已在 `Cargo.lock` 里（`sqlx` 引入），但编码不是密码学原语 ——
//! 手写 12 行可被单测钉死的 `to_hex`，比多一条依赖边更可控，
//! 也让「零新增外部 crate」这条不变式更容易用 `git diff` 验证。
//!
//! # 为什么用 SHA-256 而不是更快的哈希
//!
//! 清单里的摘要要回答的是「恢复时这份文件还是不是备份时那份」。
//! 攻击面是**离线篡改**（拿到备份目录后改文件骗过恢复校验），
//! 非加密强度要求；但摘要一旦被伪造，恢复就会静默装上错误数据
//! （`AGENTS.md` 自查表第 3 类「静默失败」），
//! 所以用抗碰撞的 SHA-256 而不是 CRC32/FNV。
//!
//! ⚠️ 边界：清单本身**不**自校验，攻击者能改文件就能改清单里的摘要。
//! 本模块提供的是**损坏检测**（bit rot / 传输截断 / 写坏），
//! **不是**防篡改认证。防篡改需要签名，属 V2 范围，不在 V1 声称的能力内。

use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

/// 分块读文件并算 SHA-256。
///
/// # 为什么分块而不是 `std::fs::read` 整读
///
/// wiki 语料 + 会话历史可能到 GB 级，整读会在 800MB 的 NAS 上直接 OOM，
/// 而**备份失败时已经写了一半文件**——留下一个看起来完整其实不全的备份目录。
/// 分块读的峰值内存固定为一个 chunk。
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    // 64 KiB：足够大以压平系统调用开销，又不至于让栈/堆压力显著。
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// 对内存中的一段字节算 SHA-256。
pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

/// 把摘要字节渲染成小写 hex。
///
/// ⚠️ 固定输出 64 个字符（SHA-256 = 32 字节）——清单解析时按**定长**校验，
/// 长度不符即判「清单损坏」，不做「尽量解析」（半截摘要放行 = 校验形同虚设）。
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 摘要的合法长度（hex 字符数）。
///
/// 定长是清单严格解析的一部分：写进清单时长度不对 = 写坏了，
/// 读回来时长度不对 = 清单被截断。两种都要判红而不是放行。
pub const DIGEST_HEX_LEN: usize = 64;

/// 判断一个字符串是否是长度正确的 hex 摘要。
pub fn is_digest_hex(s: &str) -> bool {
    s.len() == DIGEST_HEX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_published_test_vectors() {
        // NIST 公布的 SHA-256 向量。写死已知答案而不是「跑两次一致」——
        // 后者对「恒返回同一个错值」的实现同样为绿。
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_bytes(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn to_hex_is_lowercase_and_fixed_width() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xf0, 0xff]), "000ff0ff");
        assert_eq!(to_hex(&[]), "");
        // 定长不变式：任何摘要都必须是 64 个字符。
        assert_eq!(sha256_bytes(b"quill").len(), DIGEST_HEX_LEN);
    }

    #[test]
    fn is_digest_hex_rejects_wrong_shape_including_uppercase() {
        let good = sha256_bytes(b"quill");
        assert!(is_digest_hex(&good));
        // 短摘要（截断）
        assert!(!is_digest_hex(&good[..63]));
        // 多一位
        assert!(!is_digest_hex(&format!("{good}0")));
        // 大写：清单约定小写，大写说明有人手改了摘要
        assert!(!is_digest_hex(&good.to_ascii_uppercase()));
        // 非 hex 字符
        assert!(!is_digest_hex(&"z".repeat(DIGEST_HEX_LEN)));
    }

    #[test]
    fn sha256_file_matches_in_memory_variant() {
        let dir = std::env::temp_dir().join(format!("bk-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let p = dir.join("a.bin");
        // 故意大于 64 KiB chunk，强制走多轮 read，覆盖分块拼接路径。
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).expect("写临时文件");
        assert_eq!(
            sha256_file(&p).expect("算摘要"),
            sha256_bytes(&data),
            "分块读与整读结果不一致 —— 摘要在跨 chunk 边界处算错了"
        );
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_dir(&dir);
    }
}
