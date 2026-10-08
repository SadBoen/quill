//! 内容摘要（FNV-1a 变体）：**纯函数**，只做字节运算，不碰数据库。
//!
//! **为什么在内核**：`tools::mcp_tool_name` 的超长截断要拿它算 8 位后缀摘要，
//! 而内核**不能**为了一个哈希函数依赖 `quill-server::db`（层次倒挂）。
//! 本模块与 `digest32` / `digest16` 原先定义在 `quill-server::db`，
//! 2026-10-08 随 Q013 搬进内核；`db.rs` 改成 re-export，所有既有调用点
//! （`skills_repo` / `experts_repo` / `mcp_repo` / `dispatch_ledger`）零改动。
//!
//! **出处说明（最高指示第 3 条）**：这不是 goose 的移植 —— goose 没有这对函数。
//! 它是 quill 自己的产物哈希（`quill-server::db` 原有实现，一字未改地搬进来），
//! 搬家的**唯一**理由是让内核不必反向依赖壳。将来若把 `digest16` 之外的用户
//! 收干净，这里可以合并回存储层。
//!
//! 两个函数共用同一个 `fnv128` 内核：`digest32` 用两个不同种子拼出 32 字节，
//! `digest16` 直接给 16 字节。**标签参与哈希**，所以「同一个 id 用在两处」
//! 不会产出同一个摘要（`experts.asset_hash` 与 `experts.persona_hash` 正是靠这个分开）。

fn fnv128(seed: u64, parts: &[&[u8]]) -> [u8; 16] {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = seed;
    for p in parts {
        for b in *p {
            h ^= u64::from(*b);
            h = h.wrapping_mul(FNV_PRIME);
        }

        h ^= 0xff;
        h = h.wrapping_mul(FNV_PRIME);
    }
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h.to_be_bytes());
    out[8..].copy_from_slice(&h.rotate_left(32).wrapping_add(seed).to_be_bytes());
    out
}

pub fn digest32(label: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    all.push(label.as_bytes());
    all.extend_from_slice(parts);
    let a = fnv128(0x243f_6a88_85a3_08d3, &all);
    let b = fnv128(0x1319_8a2e_0370_7344, &all);
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&a);
    out[16..].copy_from_slice(&b);
    out
}

pub fn digest16(label: &str, parts: &[&[u8]]) -> [u8; 16] {
    let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    all.push(label.as_bytes());
    all.extend_from_slice(parts);
    fnv128(0x9e37_79b9_7f4a_7c15, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_label_sensitive() {
        let a = digest32("asset", &[b"cost-analyst"]);
        let b = digest32("asset", &[b"cost-analyst"]);
        let c = digest32("persona", &[b"cost-analyst"]);
        let d = digest32("asset", &[b"cost-analyst-2"]);
        assert_eq!(a, b, "同输入必须同摘要（否则 upsert 每次都写新摘要）");
        assert_ne!(a, c, "标签不同必须不同摘要");
        assert_ne!(a, d, "输入不同必须不同摘要");
    }

    #[test]
    fn digest16_distinguishes_segment_boundaries() {
        let a = digest16("dispatch-id", &[b"ab", b"c"]);
        let b = digest16("dispatch-id", &[b"a", b"bc"]);
        assert_ne!(a, b, "分段边界不同必须产出不同标识");
    }
}
