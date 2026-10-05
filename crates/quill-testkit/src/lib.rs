//! Quill 测试基础设施（公共部分）
//!
//! 依据：`docs/08_测试与验收方案.md` §3
//!
//! **本 crate 只提供公共设施，不含任何具体测试用例**（主理人明确要求）。
//! 具体断言要等 v1 恢复后才知道该测什么。
//!
//! 本目录提供五类公共设施：
//! 1. [`canary`]   —— canary 标记与测试数据工厂（所有断言"植入标记再搜标记"的基础）
//! 2. [`scan`]     —— 泄漏扫描器（三层断言的 L3 层）
//! 3. [`probe`]    —— 可观测探针（decrypt 计数、panic 捕获计数、permit 泄漏检测）
//! 4. [`mock_llm`] —— Mock LLM provider 的场景脚本模型（故障注入）
//! 5. [`mock_member`] —— `MemberExecutor` 的测试替身（P0-2 注入点，
//!    含超时 / 断链 / 拒收等委派失败模式与调用时序记录）

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod canary;
pub mod fals;
pub mod mock_llm;
pub mod mock_member;
pub mod probe;
pub mod scan;

pub use canary::{TestUser, TestUserBuilder, CANARY_PREFIX};
pub use fals::{GateRule, GateRuleSet, RuleSelfTest, SelfTestReport};
pub use mock_member::{CallRecord, FaultKind, MockMemberExecutor, Step};
pub use probe::{DecryptCounter, PanicCounter, PermitLedger};
pub use scan::{LeakHit, LeakScan};

/// 测试失败时的标准诊断信息。
///
/// **为什么强制携带 seed 与 fixture 路径**：
/// 无人 review 的环境里，AI 调试失败的唯一线索就是这三样东西。
/// 没有它们，一次失败只能靠"再跑一次看还在不在"来判断，效率极低。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDiagnostic {
    /// 确定性随机种子。失败时打印，供 `PROPTEST_CASES` 复现。
    pub seed: u64,
    /// 造数快照落盘目录。失败时人工/AI 可查看实际数据。
    pub fixture_dump: Option<std::path::PathBuf>,
    /// 具体用例 ID（如 `ISO-01`）。
    pub case_id: String,
}

impl TestDiagnostic {
    /// 生成可直接粘贴到 shell 的最小复现命令。
    pub fn repro_command(&self) -> String {
        match &self.fixture_dump {
            Some(p) => format!(
                "QUILL_CASE={} QUILL_SEED={} cargo test --test isolation -- --nocapture {}",
                self.case_id,
                self.seed,
                p.display()
            ),
            None => format!(
                "QUILL_SEED={} cargo test --test isolation -- --nocapture",
                self.seed
            ),
        }
    }
}

impl std::fmt::Display for TestDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "──────── 测试失败诊断 ─��───────")?;
        writeln!(f, "用例    : {}", self.case_id)?;
        writeln!(f, "seed    : {}", self.seed)?;
        if let Some(p) = &self.fixture_dump {
            writeln!(f, "造数快照: {}", p.display())?;
        }
        writeln!(f, "复现    : {}", self.repro_command())?;
        write!(f, "───────────────────────────────")
    }
}

/// 读取本次运行的确定性 seed。
///
/// **禁止在测试中使用 `rand::thread_rng()`** —— 那会导致失败不可复现，
/// 而无人 review 的环境下，不可复现的失败等于无法修复。
pub fn test_seed() -> u64 {
    std::env::var("QUILL_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0051_7111_CAFE) // 固定默认值，保证无 env 时也可复现
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_without_env() {
        // 两次调用必须相同，否则失败无法复现
        assert_eq!(test_seed(), test_seed());
    }

    #[test]
    fn diagnostic_prints_repro_command() {
        let d = TestDiagnostic {
            seed: 42,
            fixture_dump: Some("/tmp/fx".into()),
            case_id: "ISO-01".into(),
        };
        let s = d.to_string();
        assert!(s.contains("ISO-01"), "缺少用例 ID");
        assert!(s.contains("42"), "缺少 seed");
        assert!(s.contains("cargo test"), "缺少复现命令");
    }
}
