//! **未实现的占位 crate —— 本 crate 不含任何边界闸门逻辑。**
//!
//! # 真实边界闸门在哪（唯一真相源）
//!
//! | 诉求 | 入口 | 自证 |
//! |---|---|---|
//! | 禁止进程级单例（规则 G26-1 / G26-2） | `scripts/check-boundary-singletons.sh` | `scripts/test-check-boundary-singletons.sh` |
//! | 禁止绕过（G26-3 `include!` / G26-4 符号链接 / G26-5 `build.rs` 生成代码） | 同上 | 同上 |
//!
//! ⚠️ 上述脚本是**在服役**的边界闸门；本 crate 不是。
//!
//! # 已核实：这里没有任何可执行子命令
//!
//! ```text
//! $ cargo xtask boundary
//! error: no such command: `xtask`
//! ```
//!
//! `boundary` / `bundle-size` / `gen-workspace` 等子命令**均不存在**
//! （本 crate 无 `[[bin]]`，根 `xtask/src/` 为空目录）。
//! 文档中把它们写成可执行步骤的地方，是**尚未兑现的设计意图**，不是可跑入口。
//!
//! # 关于"边界规则数量"的说法
//!
//! 历史上出现过「六规则 / 九规则 / 11 条 / 规则 10」等互相矛盾的说法，
//! 原因是**一个未实现的命令被反复写进文档，每次措辞不同**。
//! 此处不重复任何一个数字：**数量以 `scripts/check-boundary-singletons.sh`
//! 的实际规则 ID（G26-1 ~ G26-5）为准**，不写在未实现的占位 crate 里。
//!
//! # 为什么保留这个 crate
//!
//! 它是 workspace 成员（根 `Cargo.toml` members 显式列举），
//! 移除会牵动 manifest、`Cargo.lock` 与依赖分类闸门，属独立决策。
//! 本文件只做一件事：**不再宣称本 crate 能跑边界闸门**。
//!
//! 对齐 `AGENTS.md` 铁律二十五「能被 grep 到的引用必须真实存在」。

/// 骨架占位：本 crate 目前**无公开 API**，此模块仅用于让 `cargo test --workspace`
/// 有一个明确命名的用例，**不代表任何闸门已实现**。
pub mod placeholder {
    #[cfg(test)]
    mod tests {
        /// 名刻意不叫 `crate_compiles_and_test_runs`：
        /// 旧名会让测试名看起来像"本 crate 的功能已可运行"。
        /// 本用例唯一的断言是"占位 crate 仍能编译、测试框架仍能执行"，
        /// 它**不检查**任何边界规则。
        #[test]
        fn placeholder_crate_is_unimplemented_by_design() {
            assert_eq!(
                2 + 2,
                4,
                "占位 crate 的唯一用例：证明可编译可测试，不证明任何功能"
            );
        }
    }
}
