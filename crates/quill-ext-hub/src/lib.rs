//! MCP / SKILL / 插件注册中心（配置存服务端，铁律）
//!
//! **内容归 quill-backend-engineer，骨架期只放占位。**
//!
//! ⚠️ 本文件的 doc comment 是"意图声明"，不是"已实现"——
//! 按 `AGENTS.md` 铁律二十五：**能被 grep 到的引用必须真实存在**。

/// 骨架占位：证明本 crate 能编译、能跑测试。
///
/// ⚠️ M1-1 阶段每个 crate 至少一个 `#[test]`，目的是让
/// `cargo test --workspace` 现在就跑通，
/// 而不是等所有人写完才第一次编译（那会在 M1 末爆炸）。
#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_and_test_runs() {
        assert_eq!(2 + 2, 4);
    }
}
