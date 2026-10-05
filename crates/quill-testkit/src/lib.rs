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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDiagnostic {
    pub seed: u64,

    pub fixture_dump: Option<std::path::PathBuf>,

    pub case_id: String,
}

impl TestDiagnostic {
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

pub fn test_seed() -> u64 {
    std::env::var("QUILL_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0051_7111_CAFE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_without_env() {
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
