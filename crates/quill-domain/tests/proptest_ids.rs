//! 标识 / slug 解析的**属性测试**（queue Q089）。
//!
//! 为什么单开一组属性测试，而不是再多写几条 `#[test]`：解析器的输入空间是「任意字符串」，
//! 手写的用例只能覆盖我**想得到**的那些形状；解析器恰恰死在想不到的地方（按字节下标切
//! 多字节串、把 `+001` 当数字、只判空串不判全空白）。proptest 会自己去找反例。
//!
//! **为什么为它加一个 dev-dependency**（最高指示第 4 条要求说清）：现有依赖里没有任何一个
//! 做「按策略生成输入并自动缩小反例」这件事 —— 手写循环生成随机串既不稳定也报不出最小反例。
//! proptest 是这类测试的事实标准，只进 `[dev-dependencies]`，不进产物依赖树。

use proptest::prelude::*;
use quill_domain::{TeamId, TeamIdError};

/// 合法 slug 的生成器：小写字母/数字，段间**单个**连字符，非空、不以连字符起止。
/// 与 `quill_adapters::ids::validate_slug` 的规则一一对应（长度上界 64 远大于这里的最长 38）。
fn slug() -> impl Strategy<Value = String> {
    proptest::collection::vec("[a-z0-9]{1,12}", 1..4).prop_map(|segments| segments.join("-"))
}

proptest! {
    /// 解析器不许 panic —— 输入是任意字符串，包括多字节与不可打印字符。
    #[test]
    fn team_id_parse_never_panics(s in any::<String>()) {
        let _ = TeamId::parse(&s);
    }

    /// 生成器给出的合法 slug 必须被接受，且原样保存（不做归一化）。
    #[test]
    fn a_valid_slug_parses_and_round_trips(s in slug()) {
        let id = TeamId::parse(&s).expect("按 slug 规则生成的串必须合法");
        prop_assert_eq!(id.as_str(), s.as_str());
        prop_assert_eq!(id.to_string(), s.clone());
    }

    /// 被接受的标识必须**幂等**：拿 `as_str()` 再解析一次还是它自己。
    /// 这条钉的是「parse 与 as_str 是同一套规则」——两者漂了，写进库的 id 就再也读不回来。
    #[test]
    fn an_accepted_id_reparses_identically(s in any::<String>()) {
        if let Ok(id) = TeamId::parse(&s) {
            prop_assert_eq!(id.as_str(), s.as_str());
            prop_assert!(TeamId::parse(id.as_str()).is_ok(), "已接受的标识必须能再解析一次");
        }
    }

    /// 被接受的标识只含 slug 合法字符（小写/数字/连字符）——
    /// 别让「大写能过」这种放宽悄悄溜进来。
    #[test]
    fn an_accepted_id_contains_only_slug_characters(s in any::<String>()) {
        if let Ok(id) = TeamId::parse(&s) {
            prop_assert!(
                id.as_str().bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "被接受的标识出现了非法字符：{:?}",
                id.as_str()
            );
        }
    }

    /// 全空白（空格/制表/换行）一律归到 `Empty`，不是 `Invalid`。
    #[test]
    fn a_whitespace_only_id_is_the_empty_error(s in "[ \t\r\n]+") {
        prop_assert_eq!(TeamId::parse(&s).unwrap_err(), TeamIdError::Empty);
    }
}
