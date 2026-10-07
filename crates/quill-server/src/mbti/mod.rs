//! MBTI 人格：28 题测评 → 四维光谱 → 应用到某个专家的人格正文。
//!
//! 这一块整体照 Octop 搬（`.octop-ref/octop/src/octop/api/routers/mbti.py`
//! 与 `dashboard/src/pages/Agent/Personalization/components/MBTI*.tsx`，
//! **只读对齐**），有一处是我们自己的选择：
//!
//! - Octop 把选中的类型写进 agent 的 `SOUL.md`（`mbti.py:58`）。
//!   本项目**没有 SOUL.md 这条链路**，人格正文是 `experts.instructions`
//!   （见 `mbti::apply`），所以「应用」必须指到一个具体专家身上，
//!   没有「当前智能体」这个默认目标 —— 宁可让人多选一次，
//!   也不把风格段写进一个说不清是谁的专家。

pub mod apply;
pub mod profiles;
pub mod questions;
pub mod score;
pub mod store;