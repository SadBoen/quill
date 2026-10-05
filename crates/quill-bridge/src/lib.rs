//! 实例间委派共享防护（SSRF / 路径白名单 / 头剥离）。
//!
//! # 为什么本 crate 必须独立
//!
//! **裁决依据**（`DELIVERY_PLAN.md` 十七章）：
//! 单进程架构下，`quill-server` 是「主进程」。若防护逻辑放在它里面，
//! 它就成了「持用户态载体」——而主进程按 M0 定义**不持有任何用户态**。
//! 独立 crate 使这一边界**在结构上无法被违反**。
//!
//! # 依赖方向
//!
//! - ✅ 可依赖：`quill-adapters`（注入 trait）、`quill-domain`（领域类型）
//! - ❌ **禁止**依赖 `quill-server`（会成环）
//! - ❌ **禁止**依赖 `quill-agent` / `quill-control`（边界外）
//!
//! # 三项防护
//!
//! | 防护 | 归属 | 单一真相源 |
//! |---|---|---|
//! | SSRF | 本 crate | `scripts/tunnel_policy_spec.py` |
//! | 路径白名单 | 本 crate | 同上 |
//! | 头剥离 + 强制注入 | 本 crate | 同上 |
//!
//! ⚠️ **规范目前是 Python（`scripts/tunnel_policy_spec.py`），
//! Rust 实现须与其逐条一致**——`test_tunnel_policy.py` 的 99 项用例
//! 会在迁移时作为行为基准（运维已实测 99/99 + 6/6 故障判红）。

/// 骨架占位。内容由 quill-backend（后端）与 quill-devops（运维）共同填充。
pub mod placeholder {
    /// 骨架占位：证明本 crate 能编译、能跑测试。
    #[cfg(test)]
    mod tests {
        #[test]
        fn crate_compiles_and_test_runs() {
            assert_eq!(2 + 2, 4);
        }
    }
}
