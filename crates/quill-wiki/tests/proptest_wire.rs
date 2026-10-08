//! 线格式（wire）解析的属性测试（queue Q089）：`YYYY-MM-DD` 日期与索引文件。
//!
//! 这两样都是**外部可写的文本**：日期来自页面 frontmatter 与 ingest 请求，索引来自
//! `index.md`（模型写的）。所以它们的输入空间是「任意字符串」，而不是我们枚举得出的形状。
//!
//! **为什么为它加一个 dev-dependency**：现有依赖里没有做「按策略生成输入 + 自动缩小反例」的
//! 东西。proptest 只进 `[dev-dependencies]`。

use proptest::prelude::*;
use quill_wiki::{Date, WikiIndex};

/// 日期形状的生成器：**10 字节、第 4 与第 7 字节是 `-`**，其余位置从数字 / `+` / 字母 /
/// 空格里取。为什么要专门造这个形状：`any::<String>()` 生成的随机串几乎永远在第 1 步
/// （长度不是 10）就被拒，属性看着绿其实没走到解析器里面 —— 那是「扫了个空还判绿」的变种。
fn date_shaped() -> impl Strategy<Value = String> {
    let field = |n: usize| {
        proptest::collection::vec(
            prop_oneof![
                Just('0'),
                Just('9'),
                Just('+'),
                Just('-'),
                Just('a'),
                Just(' ')
            ],
            n,
        )
        .prop_map(|cs| cs.into_iter().collect::<String>())
    };
    (field(4), field(2), field(2)).prop_map(|(y, m, d)| format!("{y}-{m}-{d}"))
}

proptest! {
    // 默认 256 例对「10 字节里挑 4 个位置、其中要有 +」这种形状命中率太低，
    // 会看着绿其实没测到。抬到 4096：解析是纯函数，代价可忽略。
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// 任意字符串都不许让日期解析 panic（粗粒度 fuzz 面）。
    #[test]
    fn date_parse_never_panics(s in any::<String>()) {
        let _ = Date::parse(&s);
    }

    /// 日期形状的输入同样不许 panic —— 这才是真能走到切片的那批输入。
    #[test]
    fn date_shaped_input_never_panics(s in date_shaped()) {
        let _ = Date::parse(&s);
    }

    /// 解析成功的日期，`Display` 必须**原样**写回输入。
    ///
    /// 这条逼出了真缺陷：`raw.parse::<u32>()` 接受前导 `+`，于是 `"+001-01-01"` 曾被
    /// 解析成 `0001-01-01`，而 `Display` 写出的是 `0001-01-01` —— 输入与输出不一致：
    /// 一个非规范写法被静默当成合法日期收下。修法是逐字段要求纯 ASCII 数字。
    #[test]
    fn a_parsed_date_renders_back_to_its_input(s in date_shaped()) {
        if let Ok(d) = Date::parse(&s) {
            prop_assert_eq!(d.to_string(), s, "解析成功就必须能原样写回（格式是定长零填充）");
        }
    }

    /// 构造出来的合法日期，`Display` 之后必须还能解析回同一个日期（读写闭环）。
    #[test]
    fn a_constructed_date_round_trips(y in 1u16..=9999, m in 1u8..=12, d in 1u8..=31) {
        if let Ok(date) = Date::new(y, m, d) {
            let text = date.to_string();
            prop_assert_eq!(Date::parse(&text).expect("Display 的输出必须可解析"), date);
        }
    }

    /// 索引文件解析器不许 panic —— 它是模型写的自由文本，fuzz 面比日期还大。
    #[test]
    fn index_parse_never_panics(text in any::<String>()) {
        let _ = WikiIndex::parse(&text);
    }
}

/// 日期字段必须是**规范写法**：纯 ASCII 数字。
///
/// 这条钉的是一个真缺陷：`raw.parse::<u32>()` 接受前导 `+`（`"+001".parse::<u32>() == Ok(1)`），
/// 所以 `"+001-01-01"` 曾被当成 `0001-01-01` 收下 —— 但 `Display` 写出的是 `0001-01-01`，
/// 输入与输出不一致：一个非规范写法被静默归一化进资料库。
#[test]
fn a_non_canonical_numeric_field_is_rejected() {
    assert!(
        Date::parse("+001-01-01").is_err(),
        "前导 `+` 不是日期字段的合法写法，必须拒绝而不是当成 0001 收下"
    );
    assert!(Date::parse("0001-+1-01").is_err());
    assert!(Date::parse("0001-01-+1").is_err());
    // 规范写法照常通过（别把这条修成「一律拒绝」）。
    assert_eq!(
        Date::parse("0001-01-01").expect("规范写法必须通过"),
        Date::new(1, 1, 1).expect("合法日期")
    );
}

/// 多字节的 10 字节串：切片安全全靠 `Date::parse` 里那两处 `b[4] == b'-' && b[7] == b'-'`。
///
/// `"ééééé"` 是 5 个两字节字符 = 10 字节，**没有** `-`：必须先被形态检查挡下。
/// 去掉那两处检查，`&s[5..7]` 会落在字符中间（字节 5 是 `é` 的续字节）而 panic ——
/// 反向验证实测过（见提交信息）。这条用固定输入把那个边界钉死，免得只靠随机撞。
#[test]
fn a_multibyte_ten_byte_string_is_rejected_by_shape_not_by_panic() {
    assert_eq!(
        Date::parse("ééééé"),
        Err(quill_wiki::date::DateError::BadShape("ééééé".to_string())),
        "没有 `-` 的 10 字节多字节串必须走形态错误，而不是 panic"
    );
    // 形态对了（字节 4/7 是 `-`）但字段是多字节：必须走「不是数字」，同样不许 panic。
    assert!(
        matches!(
            Date::parse("éé-é-é"),
            Err(quill_wiki::date::DateError::NotNumeric { .. })
        ),
        "字段不是数字要走 NotNumeric，而不是 panic"
    );
}
