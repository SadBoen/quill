
use quill_adapters::{ExpertId, MemberId, ProviderId, SessionId, UserId};

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

    let a = UserId::from_bytes([0xab; 16]);
    let b = SessionId::from_bytes([0xab; 16]);

    assert_ne!(format!("{a:?}"), format!("{b:?}"), "Debug 必须带类型名");
    assert!(format!("{a:?}").starts_with("UserId("));
    assert!(format!("{b:?}").starts_with("SessionId("));

    assert_ne!(a.to_string(), b.to_string(), "Display 必须带类型前缀");
    assert!(a.to_string().starts_with("u:"), "UserId 前缀");
    assert!(b.to_string().starts_with("s:"), "SessionId 前缀");

    assert_eq!(a.to_string()[2..], b.to_string()[2..], "同一批字节");
}

#[test]
fn member_id_and_expert_id_are_different_types_with_different_constructors() {

    let e = ExpertId::parse("cost-analyst").expect("应合法");
    let m = MemberId::for_expert(&e, 3).expect("应合法");
    assert_eq!(e.as_str(), "cost-analyst");
    assert_eq!(m.as_str(), "cost-analyst-3");

}

#[test]
fn explicit_conversion_via_string_is_possible_but_not_implicit() {

    let e = ExpertId::parse("cost-analyst").expect("应合法");
    let m = MemberId::for_expert(&e, 1).expect("应合法");

    let e2 = ExpertId::parse(m.as_str()).expect("显式重建应成功");
    assert_eq!(e2.as_str(), "cost-analyst-1");
    assert_ne!(
        e2, e,
        "重建得到的是另一个类型的值（ExpertId(-1) ≠ ExpertId）"
    );

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
