use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();

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

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub const DIGEST_HEX_LEN: usize = 64;

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

        assert_eq!(sha256_bytes(b"quill").len(), DIGEST_HEX_LEN);
    }

    #[test]
    fn is_digest_hex_rejects_wrong_shape_including_uppercase() {
        let good = sha256_bytes(b"quill");
        assert!(is_digest_hex(&good));

        assert!(!is_digest_hex(&good[..63]));

        assert!(!is_digest_hex(&format!("{good}0")));

        assert!(!is_digest_hex(&good.to_ascii_uppercase()));

        assert!(!is_digest_hex(&"z".repeat(DIGEST_HEX_LEN)));
    }

    #[test]
    fn sha256_file_matches_in_memory_variant() {
        let dir = std::env::temp_dir().join(format!("bk-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let p = dir.join("a.bin");

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
