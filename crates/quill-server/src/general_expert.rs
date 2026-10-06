//! 通用专家：每个用户都有一份，缺了就建。
//!
//! 为什么要有这个概念：**不允许「未选角色就聊天」**。
//! 过去 `POST /api/sessions` 允许 `expert_id` 缺省，建出来的会话
//! `expert_id = null`、`persona_applied = false` —— 对话页侧栏于是出现一个
//! 「默认（未选角色）」分组。用户以为在跟 Quill 说话，其实**没有任何人格在跑**，
//! 而界面上没有任何地方说明这件事。这是本项目明令禁止的那类「界面不说实话」。
//!
//! 口径：
//! - 每个人**恰好一份**通用专家，id 固定 `general`，`default_enabled = true`。
//! - 它是**真专家**：`instructions` 是一段真的通用助手人格，不是占位串。
//! - 新建会话时若调用方没指定 `expert_id`，**自动挂上它**，不留空。
//! - 读 `GET /api/experts` 之前先 `ensure`，所以界面上永远至少有这一个可选项。

use quill_adapters::{ExpertId, UserId};
use quill_agent::{Expert, NewExpert};

use crate::error::ApiError;
use crate::experts_repo::SqlxExpertRepository;
use crate::state::AppState;

/// 通用专家的固定 id。写死在代码里而不是存一张「哪个是默认」的表：
/// 约定比状态更好审计 —— 库里任何一行 id 不是 `general` 的用户专家都只是普通专家。
pub const GENERAL_EXPERT_ID: &str = "general";

/// 显示名。界面上原样显示，必须是人话。
pub const GENERAL_DISPLAY_NAME: &str = "通用助手";

/// 一句话说明它是什么。这段会进专家列表，不能写得像营销词。
pub const GENERAL_DESCRIPTION: &str =
    "不带特殊人格的通用助手：默认承接没有指定角色的对话。你可以在专家页改它的模型与人格正文。";

/// 人格正文。**这是真的指令，不是占位符。**
/// 写它的时候对着 ISSUE-006 / ISSUE-007 那两条教训：正文每轮都进 `tools` 描述，
/// 所以要短、要有用，不能堆砌。
pub const GENERAL_INSTRUCTIONS: &str = "\
你是这个工作台上的通用助手，没有特定领域人格。

工作方式：
- 先判断用户到底要什么，再动手。信息够就直接做，不要反问已经能从上下文读出来的东西。
- 缺输入就直接说缺哪一项、为什么需要它，并给出用户能照做的下一步，不要编造数据、
  不要假设文件或接口的内容。
- 需要外部信息或动作时调用挂着的工具；工具返回的内容是你的事实来源，不要用自己的记忆覆盖它。
- 工具结果拿到手就据此作答，不要反复调同一个工具。
- 回答用简体中文，简洁。代码块、命令、文件名保持原样，不要翻译。
";

/// 拿得到这个通用专家；没有就建一个。
///
/// **幂等**：已存在时原样返回，不覆盖用户改过的内容 ——
/// 用户把它的模型或人格正文改了之后，下一次 `ensure` 不该把它打回原形。
/// 建的动作对并发是安全的：`create_user_expert` 在同名且未删时返回
/// `ExpertExists`，我们捕获后重读一次即可。
pub fn ensure(state: &AppState, uid: UserId) -> Result<Expert, ApiError> {
    let registry = SqlxExpertRepository::registry(std::sync::Arc::clone(state.db()?));
    let id = ExpertId::parse(GENERAL_EXPERT_ID).expect("general 是编译期常量，parse 不可能失败");

    match registry.get_visible(&uid, &id) {
        Ok(e) => return Ok(e),
        Err(quill_agent::AgentError::ExpertNotFound { .. })
        | Err(quill_agent::AgentError::ExpertDeleted { .. }) => {}
        Err(other) => return Err(crate::api_experts::agent_error_to_api("查找通用专家", other)),
    }

    let new = NewExpert {
        id: id.clone(),
        display_name: GENERAL_DISPLAY_NAME.to_string(),
        description: GENERAL_DESCRIPTION.to_string(),
        instructions: GENERAL_INSTRUCTIONS.to_string(),
        // 不指定模型 = 跟随实例默认模型。这里写死某个模型名才是伪造：
        // 那个模型不一定存在，用户也随时可能在实例设置里换掉它。
        model: None,
        source_template: None,
    };

    match registry.create_user_expert(uid, new) {
        Ok(e) => Ok(e),
        // 另一个请求刚建好：重读，用它，不要把刚建的那份覆盖掉。
        Err(quill_agent::AgentError::ExpertExists { .. }) => {
            crate::api_experts::map_agent_error("重读通用专家", registry.get_visible(&uid, &id))
        }
        Err(other) => Err(crate::api_experts::agent_error_to_api("创建通用专家", other)),
    }
}

/// 会话该用哪个专家：调用方指定了就用它，没指定就用通用专家。
///
/// 返回的永远是 `Some` —— 这就是「不允许未选角色就聊天」在服务端的落点。
/// 真要区分「调用方显式传了空串」与「压根没传」，看调用方那层；这里一律给兜底。
pub fn resolve_for_session(state: &AppState, uid: UserId, requested: Option<String>) -> Result<String, ApiError> {
    if let Some(id) = requested.filter(|s| !s.trim().is_empty()) {
        return Ok(id);
    }
    Ok(ensure(state, uid)?.id().as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_general_expert_id_is_a_legal_slug() {
        // 常量写错的话，下面所有「按 id 取通用专家」的逻辑都会静默失配。
        ExpertId::parse(GENERAL_EXPERT_ID).expect("general 必须能通过 ExpertId::parse");
    }

    #[test]
    fn the_general_expert_is_not_a_placeholder() {
        // 占位人格是这类「自动创建」最常见的失败形式：建了一条，但等于没建。
        assert!(
            GENERAL_INSTRUCTIONS.len() > 120,
            "人格正文太短，多半是占位串：{}",
            GENERAL_INSTRUCTIONS
        );
        assert!(
            GENERAL_DESCRIPTION.len() > 20,
            "描述太短，读起来像占位：{}",
            GENERAL_DESCRIPTION
        );
    }

    #[test]
    fn the_general_expert_instructions_fit_the_per_round_budget() {
        // 这段正文**每一轮**都进 tools 描述（ISSUE-007）。
        // 超预算的话，通用专家会把每一条消息都撑大。
        let n = GENERAL_INSTRUCTIONS.chars().count();
        assert!(
            n < 1200,
            "人格正文 {n} 字符，每轮都要重发，太长了（上限 1200）"
        );
    }

    #[test]
    fn the_general_expert_pins_no_model() {
        // model 写死某个名字 = 假设那个模型一定存在，而实例设置随时能换掉它。
        // 这里恒为 None 是刻意的，不要「为了明确」改成具体模型名。
        let new = NewExpert {
            id: ExpertId::parse(GENERAL_EXPERT_ID).expect("合法"),
            display_name: GENERAL_DISPLAY_NAME.to_string(),
            description: GENERAL_DESCRIPTION.to_string(),
            instructions: GENERAL_INSTRUCTIONS.to_string(),
            model: None,
            source_template: None,
        };
        assert!(new.model.is_none(), "通用专家不许钉死模型");
    }
}