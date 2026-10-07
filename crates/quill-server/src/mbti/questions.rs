//! 测评题库：28 题、每题二选一。数据照 Octop 的 `_QUESTIONS` 搬
//! （`.octop-ref/octop/src/octop/api/routers/mbti.py:589` 处那一整段，
//! **只读对齐**：题干与选项一字不改）。
//!
//! 每题必须属于四根轴之一，且 `a_pole` / `b_pole` 正好是那根轴的两极 ——
//! 计分只认这个关系，题干的措辞不参与计算。所以要改题，只能改文字，
//! 改极性会让历史测评结果整体错位。

/// 四根轴，两极写死在这里，别处不再重复定义。
pub const AXES: [(&str, &str, &str); 4] = [
    ("EI", "E", "I"),
    ("SN", "S", "N"),
    ("TF", "T", "F"),
    ("JP", "J", "P"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Question {
    pub id: i64,
    /// `EI` / `SN` / `TF` / `JP` 之一。
    pub dimension: &'static str,
    pub a_pole: &'static str,
    pub b_pole: &'static str,
    pub question_zh: &'static str,
    pub option_a_zh: &'static str,
    pub option_b_zh: &'static str,
    pub question_en: &'static str,
    pub option_a_en: &'static str,
    pub option_b_en: &'static str,
}

/// 28 题，id 1..=28，分组顺序照 `mbti.py` 的 `_QUESTIONS`：E/I → S/N → T/F → J/P。
pub static QUESTIONS: &[Question] = &[
    // ---- E/I (7 questions) ----
    Question {
        id: 1,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "在社交活动后，你通常感到：",
        option_a_zh: "充满活力，想继续交流",
        option_b_zh: "需要独处来恢复精力",
        question_en: "After a social event, you usually feel:",
        option_a_en: "Energised and wanting to keep socialising",
        option_b_en: "Drained and needing alone time to recharge",
    },
    Question {
        id: 2,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "面对新环境时，你更倾向于：",
        option_a_zh: "主动与陌生人攀谈",
        option_b_zh: "安静观察，等待合适时机",
        question_en: "When entering a new environment, you tend to:",
        option_a_en: "Initiate conversations with strangers",
        option_b_en: "Observe quietly and wait for the right moment",
    },
    Question {
        id: 3,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "你更喜欢的工作方式是：",
        option_a_zh: "团队讨论和头脑风暴",
        option_b_zh: "独自深度思考",
        question_en: "Your preferred working style is:",
        option_a_en: "Team discussions and brainstorming",
        option_b_en: "Deep thinking alone",
    },
    Question {
        id: 4,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "理想的周末是：",
        option_a_zh: "和朋友聚会或参加活动",
        option_b_zh: "在家读书、看剧或做自己的事",
        question_en: "Your ideal weekend involves:",
        option_a_en: "Going out with friends or attending events",
        option_b_en: "Staying home reading, watching shows, or doing your own thing",
    },
    Question {
        id: 5,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "处理问题时，你倾向于：",
        option_a_zh: "先和别人讨论，边说边想",
        option_b_zh: "先自己想清楚，再和别人沟通",
        question_en: "When solving problems, you tend to:",
        option_a_en: "Talk it through with others, thinking out loud",
        option_b_en: "Think it through yourself first, then communicate",
    },
    Question {
        id: 6,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "你的朋友圈通常是：",
        option_a_zh: "广泛而多样，认识很多人",
        option_b_zh: "小而深入，几个知心朋友",
        question_en: "Your social circle is usually:",
        option_a_en: "Wide and diverse, knowing many people",
        option_b_en: "Small and deep, a few close friends",
    },
    Question {
        id: 7,
        dimension: "EI",
        a_pole: "E",
        b_pole: "I",
        question_zh: "在会议中，你更可能：",
        option_a_zh: "积极发言，分享想法",
        option_b_zh: "认真倾听，需要时再表达",
        question_en: "In meetings, you are more likely to:",
        option_a_en: "Speak up actively and share ideas",
        option_b_en: "Listen carefully and speak when needed",
    },
    // ---- S/N (7 questions) ----
    Question {
        id: 8,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "学习新事物时，你更喜欢：",
        option_a_zh: "从具体例子和实际操作开始",
        option_b_zh: "先理解整体概念和理论框架",
        question_en: "When learning something new, you prefer:",
        option_a_en: "Starting with concrete examples and hands-on practice",
        option_b_en: "Understanding the overall concept and theoretical framework first",
    },
    Question {
        id: 9,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "描述一件事时，你更倾向于：",
        option_a_zh: "关注具体细节和实际发生的事",
        option_b_zh: "描述整体印象和可能的含义",
        question_en: "When describing something, you tend to:",
        option_a_en: "Focus on specific details and what actually happened",
        option_b_en: "Describe overall impressions and possible meanings",
    },
    Question {
        id: 10,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "你更信任：",
        option_a_zh: "经过验证的经验和事实",
        option_b_zh: "直觉和内心的感悟",
        question_en: "You trust more:",
        option_a_en: "Verified experience and facts",
        option_b_en: "Intuition and inner insights",
    },
    Question {
        id: 11,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "在阅读时，你更被吸引的是：",
        option_a_zh: "实用的操作指南和说明",
        option_b_zh: "启发性的概念和隐喻",
        question_en: "When reading, you are more drawn to:",
        option_a_en: "Practical how-to guides and instructions",
        option_b_en: "Inspirational concepts and metaphors",
    },
    Question {
        id: 12,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "你更欣赏的人是：",
        option_a_zh: "脚踏实地、做事靠谱的人",
        option_b_zh: "有远见、能提出新想法的人",
        question_en: "You admire more someone who is:",
        option_a_en: "Down-to-earth and dependable",
        option_b_en: "Visionary and full of new ideas",
    },
    Question {
        id: 13,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "面对一个项目，你首先关注的是：",
        option_a_zh: "当前需要做什么，具体步骤是什么",
        option_b_zh: "这个项目最终要达到什么目标和愿景",
        question_en: "When facing a project, you first focus on:",
        option_a_en: "What needs to be done now and the specific steps",
        option_b_en: "What the ultimate goal and vision should be",
    },
    Question {
        id: 14,
        dimension: "SN",
        a_pole: "S",
        b_pole: "N",
        question_zh: "你认为自己更像是：",
        option_a_zh: "现实主义者",
        option_b_zh: "想象力丰富的人",
        question_en: "You consider yourself more of a:",
        option_a_en: "Realist",
        option_b_en: "Imaginative person",
    },
    // ---- T/F (7 questions) ----
    Question {
        id: 15,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "做重要决定时，你更依赖：",
        option_a_zh: "逻辑分析和客观标准",
        option_b_zh: "个人价值观和对他人的影响",
        question_en: "When making important decisions, you rely more on:",
        option_a_en: "Logical analysis and objective criteria",
        option_b_en: "Personal values and impact on others",
    },
    Question {
        id: 16,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "当朋友向你倾诉烦恼时，你更倾向于：",
        option_a_zh: "帮 ta 分析原因并提出解决方案",
        option_b_zh: "先表达理解和共情，陪伴 ta",
        question_en: "When a friend comes to you with a problem, you tend to:",
        option_a_en: "Analyse the cause and suggest solutions",
        option_b_en: "Express understanding and empathy first",
    },
    Question {
        id: 17,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "你更看重反馈中的：",
        option_a_zh: "直接坦诚，即使有些尖锐",
        option_b_zh: "措辞委婉，考虑对方感受",
        question_en: "In feedback, you value more:",
        option_a_en: "Direct honesty, even if a bit blunt",
        option_b_en: "Tactful wording that considers feelings",
    },
    Question {
        id: 18,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "在团队中，你更关注：",
        option_a_zh: "目标是否达成、效率是否最高",
        option_b_zh: "团队氛围是否和谐、成员是否被尊重",
        question_en: "In a team, you focus more on:",
        option_a_en: "Whether goals are met and efficiency is maximised",
        option_b_en: "Whether the atmosphere is harmonious and members feel respected",
    },
    Question {
        id: 19,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "评判一个方案时，你更看重：",
        option_a_zh: "数据和逻辑推理",
        option_b_zh: "人们的感受和接受程度",
        question_en: "When evaluating a proposal, you weigh more:",
        option_a_en: "Data and logical reasoning",
        option_b_en: "How people feel about it and their acceptance",
    },
    Question {
        id: 20,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "别人评价你时，你更希望被认为是：",
        option_a_zh: "聪明、能干、有逻辑",
        option_b_zh: "善良、温暖、体贴",
        question_en: "You would rather be seen as:",
        option_a_en: "Smart, capable, and logical",
        option_b_en: "Kind, warm, and considerate",
    },
    Question {
        id: 21,
        dimension: "TF",
        a_pole: "T",
        b_pole: "F",
        question_zh: "面对争议时，你倾向于：",
        option_a_zh: "寻找客观事实来判断对错",
        option_b_zh: "考虑每个人的立场和感受",
        question_en: "When facing a controversy, you tend to:",
        option_a_en: "Look for objective facts to judge right and wrong",
        option_b_en: "Consider everyone's position and feelings",
    },
    // ---- J/P (7 questions) ----
    Question {
        id: 22,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "你更喜欢的生活方式是：",
        option_a_zh: "有计划、有条理，按日程表行动",
        option_b_zh: "灵活随性，保持开放和弹性",
        question_en: "Your preferred lifestyle is:",
        option_a_en: "Planned, organised, following a schedule",
        option_b_en: "Flexible, spontaneous, keeping options open",
    },
    Question {
        id: 23,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "面对截止日期，你通常会：",
        option_a_zh: "提前完成，留出缓冲时间",
        option_b_zh: "在截止前才全力冲刺",
        question_en: "When facing a deadline, you usually:",
        option_a_en: "Finish early, leaving buffer time",
        option_b_en: "Sprint at full speed near the deadline",
    },
    Question {
        id: 24,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "去旅行时，你更倾向于：",
        option_a_zh: "详细规划行程和预订",
        option_b_zh: "只定大方向，到了再说",
        question_en: "When travelling, you prefer:",
        option_a_en: "Detailed itinerary planning and bookings",
        option_b_en: "Just setting a general direction and figuring it out on the go",
    },
    Question {
        id: 25,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "你的桌面或工作区域通常是：",
        option_a_zh: "整洁有序，物品各归其位",
        option_b_zh: "看似混乱但你能找到需要的东西",
        question_en: "Your desk or workspace is usually:",
        option_a_en: "Neat and organised, everything in its place",
        option_b_en: "Seemingly messy but you can find what you need",
    },
    Question {
        id: 26,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "当计划突然改变时，你：",
        option_a_zh: "感到不安，想尽快恢复秩序",
        option_b_zh: "觉得无所谓，甚至有点兴奋",
        question_en: "When plans suddenly change, you:",
        option_a_en: "Feel uneasy and want to restore order quickly",
        option_b_en: "Feel fine, maybe even a bit excited",
    },
    Question {
        id: 27,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "你做决定的速度通常是：",
        option_a_zh: "快速做出决定并执行",
        option_b_zh: "保持开放，收集更多信息再决定",
        question_en: "Your decision-making speed is usually:",
        option_a_en: "Quick to decide and execute",
        option_b_en: "Staying open, gathering more information before deciding",
    },
    Question {
        id: 28,
        dimension: "JP",
        a_pole: "J",
        b_pole: "P",
        question_zh: "你更享受的过程是：",
        option_a_zh: "完成任务打勾的满足感",
        option_b_zh: "探索各种可能性的自由感",
        question_en: "You enjoy more:",
        option_a_en: "The satisfaction of checking off completed tasks",
        option_b_en: "The freedom of exploring various possibilities",
    },
];

/// 少于这个数不算数 —— Octop 也是这个门槛（`mbti.py:642`）。
pub const MIN_ANSWERS: usize = 20;

pub fn get(id: i64) -> Option<&'static Question> {
    QUESTIONS.iter().find(|q| q.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_eight_questions_spread_evenly_over_four_axes() {
        assert_eq!(QUESTIONS.len(), 28, "题库必须正好 28 题");
        for axis in AXES {
            let n = QUESTIONS.iter().filter(|q| q.dimension == axis.0).count();
            assert_eq!(n, 7, "{} 轴应有 7 题，实得 {n}", axis.0);
        }
    }

    #[test]
    fn ids_are_unique_and_poles_match_the_axis() {
        for q in QUESTIONS {
            assert!(
                QUESTIONS.iter().filter(|o| o.id == q.id).count() == 1,
                "题号重复 {}",
                q.id
            );
            let axis = AXES
                .iter()
                .find(|a| a.0 == q.dimension)
                .unwrap_or_else(|| panic!("题 {} 的维度 {} 不存在", q.id, q.dimension));
            assert_eq!(
                (q.a_pole, q.b_pole),
                (axis.1, axis.2),
                "题 {} 的两极与轴 {} 对不上",
                q.id,
                axis.0
            );
            for text in [
                q.question_zh,
                q.option_a_zh,
                q.option_b_zh,
                q.question_en,
                q.option_a_en,
                q.option_b_en,
            ] {
                assert!(!text.trim().is_empty(), "题 {} 有空文本", q.id);
            }
        }
    }
}
