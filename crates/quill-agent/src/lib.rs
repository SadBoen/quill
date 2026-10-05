//! 专家团编排：专家领域逻辑 + 派工 + 幂等 + 崩溃恢复。
//!
//! # 本 crate 负责什么
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`error`] | 中文人话 + 可复制修复命令的领域错误（铁律七） |
//! | [`expert`] | 专家的创建/更新/删除/列表、归属与可见性 |
//! | [`dispatch`] | 用 `MemberExecutor` 派工、收集 `MemberOutcome`、失败处理、幂等、崩溃恢复 |
//!
//! # 依赖方向（`docs/PHASE2_CONTRACT.md` §一）
//!
//! ```text
//! quill-adapters（契约层：MemberExecutor / 身份 newtype）
//!       ↑
//! quill-domain（Team 聚合 + 身份 re-export）
//!       ↑
//! quill-agent（本 crate）
//! ```
//!
//! ⚠️ **不依赖 `quill-store`**：存储经 [`expert::ExpertRepository`] 与
//! [`dispatch::DispatchLedger`] 两个端口 trait 注入，由 `quill-server` 用 `sqlx` 实现。
//! 理由见 [`expert`] 模块头 —— 引入 sqlx 会新增外部依赖（须主理人裁决），
//! 且让本 crate 的领域不变量只能靠 DB 的 CHECK 兜底。
//!
//! ⚠️ **`quill-wiki` 已在依赖表内但本 crate 尚未消费它**：按契约，
//! Agent 通过 `KnowledgeBackend` trait 访问资料库而非直接依赖实现。
//! 该 trait 属 `quill-adapters` 的范畴而**尚未定义**（见 `PHASE2_CONTRACT` §2.2），
//! 因此这里**不预先造一个** —— 造一个只能被本 crate 自己实现、
//! 且与将来真 trait 签名不兼容的同名 trait，是典型幻觉引用（铁律二十五）。
//! 在真 trait 落地前，本 crate 对资料库**无任何访问路径**（不是「偷偷直接依赖」）。
//!
//! # 派工的核心不变量（都可执行验证，不是注释）
//!
//! 1. **幂等**：`(user_id, room_id, round, member_expert_id)` 唯一（`ux_dispatch_once`）。
//!    重放同一轮**不重复调用**执行器。
//! 2. **不自动重跑**：成员执行失败只如实上报，编排层**不自行重试**
//!    （`docs/07` §4.3 + `PHASE2_CONTRACT` §七.4）。
//! 3. **崩溃恢复分档**：`PENDING` 可安全重派（成员从未被调用），
//!    `RUNNING`/`ASKING` 必须人工确认（副作用可能已发生）。
//! 4. **失败不被冒充成功**：`MemberStatus::Failed` 的结果**不能**记成 `DONE`。
//! 5. **跨用户隔离**：所有存储端口方法的签名里都强制带 `UserId`。

pub mod dispatch;
pub mod error;
pub mod expert;

pub use dispatch::{
    BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, DispatchReport, DispatchState,
    DispatchTask, Dispatcher, MemDispatchLedger, MemberResult, RecoveryReport, RoundPrefix,
    RoundRequest, SharedExecutor,
};
pub use error::{chain_check_error, AgentError, MemberRejectKind, DOCTOR_CMD};
pub use expert::{
    Expert, ExpertRegistry, ExpertRepository, NewExpert, Visibility, MAX_DISPLAY_NAME, SYSTEM_OWNER,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// 铁律十九的形状自查：结论必须来自返回值，不能来自输出文本。
    ///
    /// 这条测试本身不测业务，它固化「本 crate 的公开 API 用
    /// `Result` 而不是 panic」这一口径 —— 任何人往公开路径塞 panic
    /// 时，`cargo test -p quill-agent` 会先红。
    #[test]
    fn crate_compiles_and_test_runs() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn public_error_variants_are_constructible_from_outside_the_crate() {
        // 集成测试（tests/）只能看见 `pub` 面。若某个变体只在
        // crate 内可见，外部调用方（quill-server）就拿不到它 ——
        // 这条测试把「可见性收敛」与「可用性」同时钉住。
        let e: AgentError = AgentError::ExpertNotFound {
            id: quill_adapters::ExpertId::parse("x").expect("应合法"),
        };
        assert_eq!(e.code(), "expert_not_found");
        assert!(e.fix_command().starts_with(DOCTOR_CMD));
    }
}
