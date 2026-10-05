//! Quill 定制注入点 —— 唯一允许跨层的 crate。
//!
//! # 为什么所有定制都从这里注入
//!
//! `vendor/goose/**` 是**零 diff 同步区**（见 `AGENTS.md` 铁律：未自检的闸门不算闸门
//! 之外的 D1 隔离不变式）。因此任何对 Agent 的定制都不能改上游代码，
//! 必须经由本 crate 定义的 trait 注入。
//!
//! # 依赖方向铁律
//!
//! **禁止**依赖任何其他 `quill-*` crate。
//! 它是契约层，一旦反向依赖即形成环。
//! ⚠️ 该约束由 `scripts/check-crate-deps.sh`（G40）按 `docs/DECISIONS.md`
//! D-2026-10-05-01 的裁决方向校验：`quill-adapters` 一旦依赖任何 `quill-*`，G40 判红。
//!
//! # 身份类型为什么住在契约层（`docs/DECISIONS.md` D-2026-10-05-01）
//!
//! `docs/PHASE2_CONTRACT.md` 同时要求「`quill-adapters` 零 quill 依赖」与
//! 「trait 签名直接使用 `UserId` / `ProviderId`」，而这两个类型原本归属 `quill-domain`
//! ——**两者不可兼得**。裁决：身份型 newtype 定义在本 crate，
//! `quill-domain` 依赖并 `pub use` re-export，保持 `quill_domain::UserId` 调用路径不破。
//!
//! # 本 crate 零第三方依赖
//!
//! `[dependencies]` 为空且**必须保持为空**：`async-trait` / `uuid` / `thiserror`
//! 都不在依赖表里。新增依赖会改 `Cargo.lock`，须主理人裁决。
//! 因此：`async fn in trait` 用 Rust 原生形态，`Display`/`Error` 手写。

pub mod ids;
pub mod knowledge;
pub mod member;

pub use ids::{
    validate_slug, ExpertId, MemberId, ParseIdError, ParseSlugError, ProviderId, SessionId, UserId,
    UuidBytes, MAX_SLUG_LEN,
};
pub use knowledge::{
    IndexDoc, IndexReceipt, IngestContext, KnowledgeBackend, KnowledgePage, KnowledgeSource,
    LintContext, QueryAnswer, QueryContext,
};
pub use member::{
    check_chain, AbortScope, AdapterError, ChainCheck, ChainHop, InvalidChainHop, InvalidMessage,
    InvalidOutcome, InvalidStartRequest, MemberExecutor, MemberOutcome, MemberStartRequest,
    MemberStatus, Message, MessageRole, MAX_CHAIN_DEPTH,
};
