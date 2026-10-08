pub mod dispatch;
pub mod error;
pub mod expert;
pub mod team_limits;

pub use dispatch::{
    BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, DispatchReport, DispatchState,
    DispatchTask, Dispatcher, MemDispatchLedger, MemberResult, RecoveryReport, RoundPrefix,
    RoundRequest, SharedExecutor,
};
pub use error::{chain_check_error, AgentError, MemberRejectKind, DOCTOR_CMD};
pub use expert::{
    Expert, ExpertRegistry, ExpertRepository, NewExpert, Visibility, MAX_DISPLAY_NAME,
    MAX_INSTRUCTIONS, MAX_MODEL, MAX_SOURCE_TEMPLATE, SYSTEM_OWNER,
};
pub use team_limits::{TeamLimits, TeamLimitsError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_compiles_and_test_runs() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn public_error_variants_are_constructible_from_outside_the_crate() {
        let e: AgentError = AgentError::ExpertNotFound {
            id: quill_adapters::ExpertId::parse("x").expect("应合法"),
        };
        assert_eq!(e.code(), "expert_not_found");
        assert!(e.fix_command().starts_with(DOCTOR_CMD));
    }
}
