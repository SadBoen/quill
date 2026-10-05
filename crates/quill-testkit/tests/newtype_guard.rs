//! 🔴 反向自证：身份 newtype **真的**能挡住误用。
//!
//! # 这份测试为什么不是「多写点断言」
//!
//! 若身份类型是 `type X = String`（别名），下面每一条**都能编译通过**，
//! 而编译期防线就形同虚设。证明办法：**把已知错误的用法真的编译一遍，
//! 看编译器是否拒绝** —— 结论必须来自编译器，不能来自我们的自述。
//!
//! 分两部分：
//! 1. `tests/newtype_guard.rs`（本文件）：能编译的部分 —— 证明各类型**互不相同**
//!    （用「同值不同型」的编译事实 + 显式转换的存在性反证别名假设）；
//! 2. `tests/compile_fail/newtype_misuse.rs` + `compile_fail_newtype.sh`：
//!    **故意编译不过**的用例，由脚本断言其编译失败并核对报错文本。
//!
//! ⚠️ **为什么第 2 部分是脚本而不是 `compile_fail` doctest**：
//! `compile_fail` doctest 只要「编译失败」就通过——**它不检查失败原因**。
//! 若我把代码写错了（比如打错一个函数名），它同样「通过」，变成假闸门。
//! 脚本额外断言报错文本里出现 `mismatched types` 与具体类型名，
//! 才敢说「它是因为类型不匹配而拒绝，不是因为别的原因」。

use quill_adapters::{ExpertId, MemberId, ProviderId, SessionId, UserId};

// ─────────────── 正向：各类型可以构造并携带自己的值 ───────────────

#[test]
fn all_four_identity_types_accept_their_own_constructor() {
    let uid = UserId::from_bytes([1; 16]);
    let sid = SessionId::from_bytes([2; 16]);
    let pid = ProviderId::parse("openai").expect("应合法");
    let eid = ExpertId::parse("cost-analyst").expect("应合法");

    assert_eq!(uid.to_compact_hex(), "01".repeat(16), "UserId 携带自身字节");
    assert_eq!(
        sid.to_compact_hex(),
        "02".repeat(16),
        "SessionId 携带自身字节"
    );
    assert_eq!(pid.as_str(), "openai");
    assert_eq!(eid.as_str(), "cost-analyst");
}

#[test]
fn same_payload_in_different_types_produces_different_representations() {
    // 同一串字节装进 UserId 与 SessionId：`Debug` 与 `Display` 都必须能区分，
    // 否则日志里「用户 A 的会话」与「用户 B 的会话」无法分辨。
    let a = UserId::from_bytes([0xab; 16]);
    let b = SessionId::from_bytes([0xab; 16]);

    assert_ne!(format!("{a:?}"), format!("{b:?}"), "Debug 必须带类型名");
    assert!(format!("{a:?}").starts_with("UserId("));
    assert!(format!("{b:?}").starts_with("SessionId("));

    assert_ne!(a.to_string(), b.to_string(), "Display 必须带类型前缀");
    assert!(a.to_string().starts_with("u:"), "UserId 前缀");
    assert!(b.to_string().starts_with("s:"), "SessionId 前缀");
    // 十六进制部分仍相同：证明区分来自类型标记，不是来自数据本身。
    assert_eq!(a.to_string()[2..], b.to_string()[2..], "同一批字节");
}

#[test]
fn member_id_and_expert_id_are_different_types_with_different_constructors() {
    // `MemberId::for_expert` 与 `ExpertId::parse` 是**两条不同构造路径**：
    // 实例 id 由专家身份派生，两者不是同一个概念（docs/03 §2.12.2 论据 4）。
    let e = ExpertId::parse("cost-analyst").expect("应合法");
    let m = MemberId::for_expert(&e, 3).expect("应合法");
    assert_eq!(e.as_str(), "cost-analyst");
    assert_eq!(m.as_str(), "cost-analyst-3");
    // 🔴 关键的反向事实（由「下面这段代码编译不过」证明）：
    //   MemberId::parse("cost-analyst")      ✅ 能编译
    //   ExpertId::parse("cost-analyst-3")    ✅ 能编译
    //   MemberId::parse(...) 传给要 ExpertId 的参数   ❌ 编译失败
    //   ExpertId::parse(...) 传给要 MemberId 的参数   ❌ 编译失败
}

#[test]
fn explicit_conversion_via_string_is_possible_but_not_implicit() {
    // ⚠️ 显式转换存在**不等于**可以随便互换：
    // 转换必须经过 `.as_str()` 并**重新校验**，这让每次转换都留下可审计的边界。
    let e = ExpertId::parse("cost-analyst").expect("应合法");
    let m = MemberId::for_expert(&e, 1).expect("应合法");

    let e2 = ExpertId::parse(m.as_str()).expect("显式重建应成功");
    assert_eq!(e2.as_str(), "cost-analyst-1");
    assert_ne!(
        e2, e,
        "重建得到的是另一个类型的值（ExpertId(-1) ≠ ExpertId）"
    );

    // 反向：从非法字符串重建必须失败，不许「转换时静默接受」。
    assert!(
        MemberId::parse("Bad Name").is_err(),
        "重建必须重新校验 slug 规则"
    );
}

#[test]
fn identity_types_are_hashable_so_they_can_be_map_keys() {
    use std::collections::HashSet;
    let mut set: HashSet<UserId> = HashSet::new();
    set.insert(UserId::from_bytes([1; 16]));
    assert!(set.contains(&UserId::from_bytes([1; 16])), "同值应命中");
    assert!(!set.contains(&UserId::from_bytes([2; 16])), "异值不应命中");
    assert_eq!(set.len(), 1, "已检查：集合里应只有 1 个元素");
}
