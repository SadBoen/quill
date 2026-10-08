//! `HubError`：上游调用失败的四种结局，与「六个榜单全挂」时的聚合规则。
//!
//! 这一支为什么必须存在、为什么 TooLarge 不能塞回 Fetch，见文件内各处注释与
//! `mod.rs` 的模块文档（「报错要说清哪一样坏了」）。

use super::endpoints::showcase_path;
use super::http::host;

/// 上游调用失败。**区分「网络/上游坏了」与「它说没有」** ——
/// 这两件事在界面上的说法必须不同。
#[derive(Debug)]
pub enum HubError {
    /// 请求本身失败（超时、TLS、5xx）。**没有拿到任何可信内容。**
    Fetch(String),
    /// 拿到了但不是 JSON，或结构对不上。**同样不能当成「没有技能」。**
    Parse(String),
    /// 上游明确返回的 4xx。
    Status(u16),
    /// **调用方给的值不对**（slug 含非法字符、榜单名不存在）。
    ///
    /// 为什么要与 `Parse` 分开：这两者的「下一步」完全相反。
    /// `Parse` 是「上游改了接口，重试没用」；
    /// 而 slug 非法时，用户只要**换一个名字**就能继续 ——
    /// 把它说成「上游坏了」，等于让用户去查一件根本没坏的东西。
    Input(String),
    /// **响应体超过 `MAX_HTTP_BYTES`，被我们在读的过程中拒收。**
    ///
    /// ## 为什么必须是独立的一个变体
    ///
    /// 这一条以前是塞进 [`HubError::Fetch`] 的，于是界面上说的是
    /// 「连不上技能市场……下一步：检查网络，或换个镜像」。
    /// **那个下一步在这件事上全错**：镜像是活的，它确实回了东西，
    /// 只不过回得比 [`MAX_HTTP_BYTES`] 大。用户去查网络、查镜像，
    /// 查的是一件根本没坏的东西 —— 这正是本文件开头那条原则
    /// （把「上游坏了」当结论，等于让用户去调查从未出过问题的地方）。
    ///
    /// 所以长度违规是**独立的一支**，不是传输错误的一种口味。
    /// `Fetch` 保留给真正的传输层失败（连不上、超时、TLS），
    /// `Status` 保留给非 2xx，三者加上这一支共四种，界面各自有话说。
    TooLarge {
        /// 是哪个接口超了（URL 的 path，或 zip 的 slug）。
        label: String,
        /// 当时的字节上限，**原样带出来** —— 用户要拿它去判断
        /// 「换一个响应更小的市场有没有意义」，不给数字他只能猜。
        limit: u64,
    },
}

impl HubError {
    /// 面向用户的一句话 + 下一步。**永远给得出下一步**。
    pub fn message(&self) -> String {
        match self {
            HubError::Fetch(detail) => format!(
                "连不上技能市场（{}）。下一步：检查网络，或用 QUILL_SKILLHUB_HOST \
                 指向一个可达的 SkillHub 镜像。详情：{detail}",
                host()
            ),
            HubError::Parse(detail) => format!(
                "技能市场的响应看不懂（不是预期的 JSON）。下一步：这是上游改了接口，\
                 本页暂时不能用；已有的技能不受影响。详情：{detail}"
            ),
            HubError::Status(code) => match code {
                // 404 与 5xx 要分开说。混成一句「稍后重试」的话，
                // 用户会对一个**根本不存在**的技能重试到天荒地老。
                404 => "技能市场里没有这个技能（上游返回 404）。下一步：回到列表里重新选一个；\
                        也可能是它刚被下架。"
                    .to_string(),
                401 | 403 => "技能市场拒绝了这次请求（需要凭据或被限流）。下一步：\
                              稍后重试；若持续出现，用 QUILL_SKILLHUB_HOST 换一个可用的市场地址。"
                    .to_string(),
                _ => format!(
                    "技能市场返回了 HTTP {code}。下一步：稍后重试；\
                     若持续出现，用 QUILL_SKILLHUB_HOST 换一个可用的市场地址。"
                ),
            },
            HubError::Input(detail) => {
                format!("{detail}。下一步：换一个名字再来，或回到列表里重新选一个。")
            }
            // 措辞上刻意**不说「连不上」也不说「检查网络」**：市场好好地回了，
            // 只是回得太大。指向网络或让用户换镜像都是指错方向 —— 那两件事
            // 在这里都帮不上忙。给的下一步只能是「换一个响应更小的来源」。
            HubError::TooLarge { label, limit } => format!(
                "技能市场的 {label} 太大了：响应体超过我们单次接收的上限 \
                 {mib} MiB（来自 {host}）。下一步：这一个技能包或列表超限，\
                 换一个更小的技能试试；若持续出现，用 QUILL_SKILLHUB_HOST \
                 指向一个响应更小的 SkillHub 镜像，或把上限调高。详情：读到 \
                 {limit} 字节上限就停（是超限中止，不是传输中断）。",
                mib = limit / 1024 / 1024,
                host = host(),
            ),
        }
    }

    /// 是不是「响应体太大」这一支。
    ///
    /// 给只做「错误 → HTTP 状态码」映射、拿不到变体的调用方用：
    /// 它们能问出「这是超限」，而不是退回到「上游不可用」那一档。
    pub fn is_oversize(&self) -> bool {
        matches!(self, HubError::TooLarge { .. })
    }
}

/// 6 个榜单全挂时，把它们各自的结局**合成一个**，且**不丢类**。
///
/// ## 规则：只要有一个超限，聚合结果就是超限
///
/// 混因（一部分超限、一部分真的传输失败）也走这一条，**不按数量多数决**。
///
/// ## 为什么是「有一个就够」
///
/// - 只要**有一个**榜单收到了超限响应，链路就被证明是通的：`host` 可达、
///   镜像可达、上游确实在回话。于是 `SKILLHUB_ADVICE`（先确认网络能到上游）
///   那一档的前提当场被证伪 —— 拿它当这一步的「下一步」，是把用户送去查网络。
/// - 1 个超限 / 5 个「连不上」里，那 5 个「连不上」很可能是**同一个大响应**
///   把上游或中间 CDN 的连接压垮的**后果**，不是两件独立的故障。
///   先说超限更接近根因，而下一步（换更小的来源或调上限）对两者都成立。
/// - 多数决的坏处就在这里：1 超限 / 5 连不上时它会选「连不上」，
///   正好把用户送去查一件被那 1 个超限证伪了的东西。
///
/// ## 边界：一个超限都没有时，仍然是 `Fetch`
///
/// 全是传输 / 解析 / 状态码错误，就还是「够不着」，网络建议照旧。
/// 这一条要钉住：别把这个缺口修成「谁都不再说网络」。
pub(super) fn aggregate_board_errors(failures: &[(String, HubError)]) -> HubError {
    // 判定走变体本身，**不拿中文认字**：认字的判据会跟着文案改而悄悄失效，
    // 而把「太大」压成「Fetch」最初就是这样塌掉的。
    let limit = failures.iter().find_map(|(_, e)| match e {
        HubError::TooLarge { limit, .. } => Some(*limit),
        _ => None,
    });
    match limit {
        Some(limit) => HubError::TooLarge {
            label: board_label(failures),
            limit,
        },
        // 没有超限 = 真的是「够不着」：各榜单自己的失败详情原样串起来。
        None => HubError::Fetch(format!(
            "六个榜单一个都没拉到：{}",
            failures
                .iter()
                .map(|(k, e)| format!("{k}：{}", e.message()))
                .collect::<Vec<_>>()
                .join("；")
        )),
    }
}

/// 聚合超限时给 `label` 的一句话：**哪几个**榜单、各自**为什么**没的。
///
/// 不用各榜单自己的 `message()`：那句 `Fetch` 的话术里带着
/// 「下一步：检查网络，或用 QUILL_SKILLHUB_HOST 指向一个**可达的**镜像」，
/// 把它塞进一个 `TooLarge` 的详情里，聚合结果就自相矛盾 ——
/// 那正是这个缺口在修的那件事。底层那串细节（`connection reset` 之类）
/// 仍然原样带出，所以「为什么」一样没丢。
fn board_label(failures: &[(String, HubError)]) -> String {
    let paths = failures
        .iter()
        .map(|(k, _)| showcase_path(k).unwrap_or(k.as_str()).to_string())
        .collect::<Vec<_>>()
        .join("、");
    if failures.iter().all(|(_, e)| e.is_oversize()) {
        // 全是超限时，每个榜单的理由与聚合结果本身是同一句话，
        // 重复六遍只是噪音；点名是哪六个接口就够用户去比了。
        return format!(
            "{paths}（这 {n} 个榜单都因响应体超过上限被拒收）",
            n = failures.len()
        );
    }
    let reasons = failures
        .iter()
        .map(|(k, e)| format!("{k}：{}", board_reason(e)))
        .collect::<Vec<_>>()
        .join("；");
    format!("{paths}（逐个：{reasons}）")
}

/// 单个榜单失败的一句话理由，**短**，且不替聚合结果做主张。
fn board_reason(e: &HubError) -> String {
    match e {
        HubError::TooLarge { limit, .. } => format!("响应体超上限 {limit} 字节"),
        // 刻意不写「连不上」：这个聚合结果的类**不是**传输失败，
        // 这里冒出一个「连不上」就是在给一个超限结局配网络口径的话。
        HubError::Fetch(d) => format!("传输失败（{d}）"),
        HubError::Parse(d) => format!("响应不是预期 JSON（{d}）"),
        HubError::Status(c) => format!("上游返回 HTTP {c}"),
        HubError::Input(d) => format!("请求值不对（{d}）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skillhub::endpoints::showcase_path;

    use crate::skillhub::{MAX_HTTP_BYTES, SHOWCASE_KINDS};

    // -----------------------------------------------------------------------
    // 错误文案：**每一条都得说对下一步**
    //
    // 真机实测（2026-10-06）踩到过两个真问题，下面就是它们的钉子：
    // 1. 非法 slug 被报成「上游改了接口」—— 用户会去查一个没坏的上游；
    // 2. 技能 404 的「下一步」写着「启动 llama-server」—— 与技能市场毫无关系。
    // -----------------------------------------------------------------------

    #[test]
    fn a_rejected_slug_is_blamed_on_the_caller_not_on_the_market() {
        // 第一版的锅：`validate_slug` 返回 `Parse`，渲染出来是
        // 「技能市场的响应看不懂…这是上游改了接口」。用户真的会去重启上游。
        let m = HubError::Input("技能标识 \"A_B\" 含不允许的字符".to_string()).message();
        assert!(m.contains("换一个名字"), "{m}");
        assert!(!m.contains("上游"), "用户的输入问题不该赖到上游头上：{m}");
        assert!(!m.contains("接口"), "{m}");
    }

    #[test]
    fn a_404_is_its_own_story_and_not_a_connect_failure() {
        // 「市场里没有这个」与「连不上市场」在界面上必须是两句话。
        // 混成一句「稍后重试」，用户会对着一个不存在的技能重试到天荒地老。
        let m = HubError::Status(404).message();
        assert!(m.contains("没有这个技能"), "{m}");
        assert!(m.contains("下架"), "要说清它可能是被下架了：{m}");
        assert!(!m.contains("重试"), "对不存在的技能说「重试」是错的路：{m}");

        let bad = HubError::Status(500).message();
        assert!(bad.contains("重试"), "5xx 才是该重试的：{bad}");
    }

    #[test]
    fn a_missing_credential_and_a_server_error_are_not_the_same_advice() {
        // 401/403 是「这个市场要凭据」，5xx 是「它自己坏了」。
        let denied = HubError::Status(403).message();
        let broke = HubError::Status(500).message();
        assert!(denied.contains("凭据"), "{denied}");
        assert!(!broke.contains("凭据"), "5xx 与凭据无关：{broke}");
    }

    /// 这次修复针对的正是这条错路：**上一句「下一步」在指向没坏的东西**。
    ///
    /// 市场活着、回了个 33 MiB 的包，用户却被叫去查网络。断言的方向要**反过来**：
    /// 超限那句话里**不能**有连不上的那句建议。
    #[test]
    fn an_oversized_response_is_not_reported_as_a_network_failure() {
        let oversize = HubError::TooLarge {
            label: "/api/v1/skillsets/big/download".to_string(),
            limit: MAX_HTTP_BYTES,
        }
        .message();

        // 1. 不能出现「连不上技能市场」—— 镜像是通的，它只是回得太大。
        assert!(
            !oversize.contains("连不上"),
            "超限不是连不上，说成连不上就是让用户去查网络：{oversize}"
        );
        // 2. 不能出现连不上那句独有的建议。
        assert!(
            !oversize.contains("检查网络"),
            "网络在这里是好的：{oversize}"
        );
        assert!(
            !oversize.contains("可达的 SkillHub 镜像"),
            "「可达」是连不上时才有的说法：{oversize}"
        );
        // 3. 但它得说得清真的坏了什么：上限与来源都要给。
        assert!(
            oversize.contains("32") && oversize.contains("MiB"),
            "超限要给出具体上限，用户才知道该拿什么去比：{oversize}"
        );
        assert!(
            oversize.contains(&host()),
            "要说出是哪个来源回的这么大：{oversize}"
        );
        assert!(
            oversize.contains("/api/v1/skillsets/big/download"),
            "要说出是哪个接口：{oversize}"
        );
        assert!(
            oversize.contains("下一步"),
            "每条错误都要有下一步：{oversize}"
        );

        // 4. 两句必须**真的不同**，不是同一句话的两种写法。
        let transport = HubError::Fetch("connection reset".into()).message();
        assert!(
            transport.contains("检查网络"),
            "真正的传输失败才该说「检查网络」：{transport}"
        );
        assert_ne!(
            transport, oversize,
            "超限与连不上合成一句话，用户就没法知道该查哪一样"
        );
        assert!(
            !oversize.contains("connection reset"),
            "超限那支里不该混进别的错误的细节：{oversize}"
        );
    }

    /// 四种结局各自可辨 —— 调用方不用去猜一句中文。
    #[test]
    fn the_four_outcomes_are_four_distinguishable_branches() {
        let transport = HubError::Fetch("timeout".into());
        let bad_status = HubError::Status(502);
        let oversize = HubError::TooLarge {
            label: "/api/v1/showcase/hot".to_string(),
            limit: MAX_HTTP_BYTES,
        };
        let parse = HubError::Parse("expected skillSets".into());

        assert!(!transport.is_oversize());
        assert!(!bad_status.is_oversize(), "5xx 不是超限");
        assert!(!parse.is_oversize(), "解析失败不是超限");
        assert!(oversize.is_oversize());

        // 非 2xx 说的是状态码，超限说的是体积：不能互相顶替。
        assert!(bad_status.message().contains("502"));
        assert!(oversize.message().contains("MiB"));
        assert_ne!(bad_status.message(), oversize.message());
    }

    /// 六个榜单全因超限而空时，聚合出来的错误**仍然是超限那一支**。
    ///
    /// 这条钉住的是聚合这一步：`showcase_all` 原先把每个榜单的结局压成一个
    /// `Fetch(String)`，类没了，调用方于是给出「先确认网络能到上游」——
    /// 而六个榜单都是上游好好回话、只是回得太大。
    #[test]
    fn boards_that_all_failed_on_size_aggregate_to_too_large_not_to_a_network_failure() {
        let failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| {
                (
                    k.to_string(),
                    HubError::TooLarge {
                        label: showcase_path(k).unwrap_or_default().to_string(),
                        limit: MAX_HTTP_BYTES,
                    },
                )
            })
            .collect();
        assert_eq!(failures.len(), 6, "六个榜单都要在");

        let err = aggregate_board_errors(&failures);
        assert!(
            err.is_oversize(),
            "全因超限而空，聚合结果不能变成传输失败：{err:?}"
        );
        let m = err.message();
        for wrong in ["连不上", "检查网络", "先确认网络能到上游", "可达的"] {
            assert!(
                !m.contains(wrong),
                "聚合结果里出现「{wrong}」，用户就会去查一件被证伪了的东西：{m}"
            );
        }
        // 类保住了，但不能因此把「哪几个」和「为什么」丢了。
        for k in SHOWCASE_KINDS.iter().filter(|k| **k != "all") {
            let p = showcase_path(k).unwrap_or_default();
            assert!(m.contains(p), "要点名是哪个接口超了：{m}");
        }
        assert!(m.contains("32") && m.contains("MiB"), "上限要给数字：{m}");
        assert!(m.contains(&host()), "要说出是哪个来源回的这么大：{m}");
    }

    /// 混因的规则：**有一个超限就算超限**，不按数量多数决。
    ///
    /// 这条是**故意选的**规则，所以把被否掉的那条也钉在这里：
    /// 1 个超限 + 5 个传输失败，多数决会选「连不上」，于是给网络建议 ——
    /// 而那 1 个超限已经把「网络不通」证伪了。
    #[test]
    fn a_single_oversized_board_outvotes_many_transport_failures() {
        let mut failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| (k.to_string(), HubError::Fetch("connection reset".into())))
            .collect();
        failures[0].1 = HubError::TooLarge {
            label: "/api/v1/showcase/recommended".into(),
            limit: MAX_HTTP_BYTES,
        };
        let oversize_count = failures.iter().filter(|(_, e)| e.is_oversize()).count();
        let transport_count = failures.len() - oversize_count;
        assert!(
            transport_count > oversize_count,
            "这一条要检的正是「多数其实是传输失败」的情形：{oversize_count} 超限 / {transport_count} 传输"
        );

        let err = aggregate_board_errors(&failures);
        assert!(
            err.is_oversize(),
            "多数决会把用户送去查网络，而那 1 个超限证明网络是通的：{err:?}"
        );
        let m = err.message();
        for wrong in ["连不上", "检查网络", "先确认网络能到上游", "可达的"] {
            assert!(!m.contains(wrong), "混因时也不能给网络口径的话：{m}");
        }
        // 反过来：传输失败那一路**也要说清楚**，否则用户只看到「都太大」，
        // 而其实还有几个是真的够不着。
        assert!(
            m.contains("connection reset"),
            "各榜单的细节不能被吞掉：{m}"
        );
        assert!(
            m.contains("传输失败"),
            "混因时要逐个点名，传输失败那一支不能被超限盖住：{m}"
        );
    }

    /// 反向也钉住：**一个超限都没有**时，网络建议必须照旧。
    ///
    /// 防的是把这个缺口修过头 —— 变成「谁都不再说网络」是新的一处误导。
    #[test]
    fn boards_that_all_failed_for_transport_reasons_still_get_the_network_advice() {
        let failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| (k.to_string(), HubError::Fetch("connection reset".into())))
            .collect();
        let err = aggregate_board_errors(&failures);
        assert!(!err.is_oversize(), "全是传输失败，不是超限：{err:?}");
        let m = err.message();
        assert!(
            m.contains("检查网络"),
            "真的够不着时仍然要说网络，不然用户没有下一步：{m}"
        );
        assert!(
            m.contains("connection reset"),
            "各榜单的失败详情要原样带着：{m}"
        );
    }

    /// 非超限、非传输的结局**照样**留在聚合里，不能被「有一个超限就全算超限」抹掉。
    #[test]
    fn a_mixed_aggregate_names_every_board_reason() {
        let failures = vec![
            (
                "recommended".to_string(),
                HubError::TooLarge {
                    label: "/api/v1/showcase/recommended".into(),
                    limit: MAX_HTTP_BYTES,
                },
            ),
            ("paid".to_string(), HubError::Status(503)),
            ("newest".to_string(), HubError::Parse("缺 skills".into())),
            ("hot".to_string(), HubError::Fetch("timeout".into())),
        ];
        let err = aggregate_board_errors(&failures);
        assert!(err.is_oversize());
        let m = err.message();
        for want in [
            "/api/v1/showcase/recommended",
            "503",
            "缺 skills",
            "timeout",
        ] {
            assert!(m.contains(want), "每个榜单各自的结局都要在聚合里：{m}");
        }
        for wrong in ["连不上", "检查网络", "可达的"] {
            assert!(!m.contains(wrong), "聚合类是超限，不该混进网络口径：{m}");
        }
    }

    #[test]
    fn every_failure_has_a_next_step_and_no_advice_talks_about_the_model_server() {
        // 兜底检查：这几条是用户唯一会照着做的内容，
        // 一旦混进模型服务的话术，后果比不给建议更糟。
        let all = [
            HubError::Fetch("timeout".into()).message(),
            HubError::Parse("bad json".into()).message(),
            HubError::Status(404).message(),
            HubError::Status(500).message(),
            HubError::Status(401).message(),
            HubError::Input("slug".into()).message(),
            HubError::TooLarge {
                label: "/api/v1/search".into(),
                limit: MAX_HTTP_BYTES,
            }
            .message(),
        ];
        for m in &all {
            assert!(m.contains("下一步"), "这句没有下一步：{m}");
            assert!(!m.contains("llama-server"), "这句在扯模型服务：{m}");
            assert!(!m.contains("QUILL_LLM_BASE_URL"), "这句在扯模型服务：{m}");
        }
    }
}
