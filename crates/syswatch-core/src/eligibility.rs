//! Conservative process eligibility based on one scan's provenance.
//!
//! This assessment does not infer abandonment from parent PID 1, CPU, or
//! lifetime. `TerminalAssociation::NotObserved` means evidence was unavailable,
//! not that the process lacks a terminal.

use std::collections::{BTreeMap, BTreeSet};

use crate::provenance::{ParentContext, ProcessProvenance};

/// UIDs below this threshold are treated as system owners.
///
/// This intentionally over-protects some Linux user accounts.
pub const SYSTEM_UID_THRESHOLD: u32 = 500;
/// Maximum number of validated parent links inspected in one scan.
pub const MAX_ANCESTRY_DEPTH: u32 = 32;
/// Program labels treated as terminal, multiplexer, or login-shell evidence.
///
/// GUI editors are excluded because their reported `argv[0]` is unstable.
pub const TERMINAL_PROGRAMS: &[&str] = &[
    "kitty",
    "Terminal",
    "iTerm2",
    "Alacritty",
    "alacritty",
    "WezTerm",
    "wezterm-gui",
    "Hyper",
    "Warp",
    "ghostty",
    "tmux",
    "screen",
    "zellij",
    "zsh",
    "bash",
    "fish",
    "sh",
    "dash",
];

/// Coarse result of conservative process-context assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityState {
    /// Evidence identifies a protected process or context.
    Protected,
    /// Owner and complete ancestry evidence support later advisory analysis.
    Eligible,
    /// Available evidence is incomplete or insufficient.
    Unknown,
}

/// Evidence supporting a protected or uncertain assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityReason {
    /// The assessed identity is the monitor process.
    MonitorProcess,
    /// A validated ancestor is the monitor process.
    MonitorAncestor,
    /// The process runs as the privileged UID zero.
    PrivilegedOwner,
    /// The process owner UID falls below [`SYSTEM_UID_THRESHOLD`].
    SystemOwner,
    /// A validated ancestor has a recognized terminal or shell label.
    ActiveTerminal,
    /// The assessed identity was absent from the scan.
    ProvenanceUnavailable,
    /// The process owner UID was unavailable.
    OwnerUnknown,
    /// A parent identity could not be validated in the scan.
    AncestryIncomplete,
    /// The validated ancestry contains a cycle.
    AncestryCycle,
    /// The ancestry exceeded [`MAX_ANCESTRY_DEPTH`].
    AncestryTooDeep,
}

/// Result of walking validated parent identities in one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AncestryOutcome {
    /// The chain reached a row with no reported parent.
    ReachedRoot,
    /// A resolved parent identity was absent from the scan.
    MissingAncestor { identity: (u32, u64) },
    /// Only a parent PID was reported; its identity was not followed.
    UnvalidatedParent { pid: u32 },
    /// A validated identity repeated in the chain.
    Cycle { identity: (u32, u64) },
    /// More than the allowed number of validated links was required.
    DepthExceeded,
}

/// Validated ancestry length and its stopping condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ancestry {
    /// Number of validated parent links followed.
    pub hops: u32,
    /// Why the walk stopped.
    pub outcome: AncestryOutcome,
}

/// Positive evidence of a terminal-related ancestor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAssociation {
    /// A validated ancestor matched a known program label.
    Observed {
        identity: (u32, u64),
        program: String,
    },
    /// No validated ancestor matched a known program label.
    NotObserved,
}

/// Eligibility result for one process identity and one provenance scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityAssessment {
    /// Assessed `(pid, start_time_secs)` identity.
    pub identity: (u32, u64),
    /// Conservative classification.
    pub state: EligibilityState,
    /// Reasons in the fixed precedence order.
    pub reasons: Vec<EligibilityReason>,
    /// Observed owner UID, if available.
    pub owner_uid: Option<u32>,
    /// Validated parent-chain result, if the process was in the scan.
    pub ancestry: Option<Ancestry>,
    /// Positive terminal evidence, or `NotObserved`.
    pub terminal: TerminalAssociation,
}

/// Assesses one identity using only provenance from the latest scan.
pub fn assess(
    identity: (u32, u64),
    scan: &BTreeMap<(u32, u64), ProcessProvenance>,
    monitor: Option<(u32, u64)>,
) -> EligibilityAssessment {
    let Some(process) = scan.get(&identity) else {
        return EligibilityAssessment {
            identity,
            state: EligibilityState::Unknown,
            reasons: vec![EligibilityReason::ProvenanceUnavailable],
            owner_uid: None,
            ancestry: None,
            terminal: TerminalAssociation::NotObserved,
        };
    };

    let owner_uid = process.owner_uid;
    let mut visited = BTreeSet::from([identity]);
    let mut ancestors = Vec::new();
    let mut current = process;
    let mut hops = 0;
    let outcome = loop {
        match current.parent {
            ParentContext::NoParent => break AncestryOutcome::ReachedRoot,
            ParentContext::PidOnly(pid) => break AncestryOutcome::UnvalidatedParent { pid },
            ParentContext::Resolved {
                pid,
                start_time_secs,
            } => {
                let parent = (pid, start_time_secs);
                if visited.contains(&parent) {
                    break AncestryOutcome::Cycle { identity: parent };
                }
                if hops == MAX_ANCESTRY_DEPTH {
                    break AncestryOutcome::DepthExceeded;
                }
                let Some(row) = scan.get(&parent) else {
                    break AncestryOutcome::MissingAncestor { identity: parent };
                };
                visited.insert(parent);
                ancestors.push(parent);
                hops += 1;
                current = row;
            }
        }
    };

    let terminal = ancestors
        .iter()
        .filter_map(|ancestor| {
            let program = scan.get(ancestor)?.program.as_ref()?;
            TERMINAL_PROGRAMS
                .iter()
                .any(|known| program.trim_start_matches('-').eq_ignore_ascii_case(known))
                .then(|| TerminalAssociation::Observed {
                    identity: *ancestor,
                    program: program.clone(),
                })
        })
        .next()
        .unwrap_or(TerminalAssociation::NotObserved);
    let ancestry = Ancestry { hops, outcome };

    let mut reasons = Vec::new();
    if monitor == Some(identity) {
        reasons.push(EligibilityReason::MonitorProcess);
    }
    if monitor.is_some_and(|monitor| ancestors.contains(&monitor)) {
        reasons.push(EligibilityReason::MonitorAncestor);
    }
    if owner_uid == Some(0) {
        reasons.push(EligibilityReason::PrivilegedOwner);
    }
    if owner_uid.is_some_and(|uid| uid < SYSTEM_UID_THRESHOLD) {
        reasons.push(EligibilityReason::SystemOwner);
    }
    if matches!(terminal, TerminalAssociation::Observed { .. }) {
        reasons.push(EligibilityReason::ActiveTerminal);
    }
    if !reasons.is_empty() {
        return EligibilityAssessment {
            identity,
            state: EligibilityState::Protected,
            reasons,
            owner_uid,
            ancestry: Some(ancestry),
            terminal,
        };
    }

    if owner_uid.is_none() {
        reasons.push(EligibilityReason::OwnerUnknown);
    }
    match ancestry.outcome {
        AncestryOutcome::MissingAncestor { .. } | AncestryOutcome::UnvalidatedParent { .. } => {
            reasons.push(EligibilityReason::AncestryIncomplete);
        }
        AncestryOutcome::Cycle { .. } => reasons.push(EligibilityReason::AncestryCycle),
        AncestryOutcome::DepthExceeded => reasons.push(EligibilityReason::AncestryTooDeep),
        AncestryOutcome::ReachedRoot => {}
    }
    let state = if reasons.is_empty() {
        EligibilityState::Eligible
    } else {
        EligibilityState::Unknown
    };

    EligibilityAssessment {
        identity,
        state,
        reasons,
        owner_uid,
        ancestry: Some(ancestry),
        terminal,
    }
}

/// Returns the current process identity when it appears in the scan.
pub fn monitor_identity(scan: &BTreeMap<(u32, u64), ProcessProvenance>) -> Option<(u32, u64)> {
    let pid = std::process::id();
    scan.keys()
        .find(|(candidate, _)| *candidate == pid)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(
        pid: u32,
        start_time_secs: u64,
        parent: ParentContext,
        program: Option<&str>,
        owner_uid: Option<u32>,
    ) -> ProcessProvenance {
        ProcessProvenance {
            pid,
            start_time_secs,
            executable: None,
            program: program.map(str::to_owned),
            owner_uid,
            parent,
        }
    }

    fn scan(rows: Vec<ProcessProvenance>) -> BTreeMap<(u32, u64), ProcessProvenance> {
        rows.into_iter().map(|row| (row.identity(), row)).collect()
    }

    fn resolved(pid: u32, start_time_secs: u64) -> ParentContext {
        ParentContext::Resolved {
            pid,
            start_time_secs,
        }
    }

    #[test]
    fn missing_provenance_is_unknown() {
        let result = assess((1, 1), &scan(vec![]), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_eq!(
            result.reasons,
            vec![EligibilityReason::ProvenanceUnavailable]
        );
        assert_eq!(result.owner_uid, None);
        assert_eq!(result.ancestry, None);
    }

    #[test]
    fn monitor_process_is_protected() {
        let row = provenance(2, 1, ParentContext::NoParent, None, Some(501));
        let result = assess(row.identity(), &scan(vec![row]), Some((2, 1)));
        assert_eq!(result.state, EligibilityState::Protected);
        assert_eq!(result.reasons, vec![EligibilityReason::MonitorProcess]);
    }

    #[test]
    fn monitor_ancestor_is_protected() {
        let rows = vec![
            provenance(2, 1, resolved(3, 1), None, Some(501)),
            provenance(3, 1, resolved(4, 1), None, Some(501)),
            provenance(4, 1, ParentContext::NoParent, None, Some(501)),
        ];
        let result = assess((2, 1), &scan(rows), Some((4, 1)));
        assert_eq!(result.state, EligibilityState::Protected);
        assert_eq!(result.reasons, vec![EligibilityReason::MonitorAncestor]);
    }

    #[test]
    fn privileged_owner_is_protected() {
        let row = provenance(2, 1, ParentContext::NoParent, None, Some(0));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(result.state, EligibilityState::Protected);
        assert_eq!(
            result.reasons,
            vec![
                EligibilityReason::PrivilegedOwner,
                EligibilityReason::SystemOwner
            ]
        );
    }

    #[test]
    fn system_uid_owner_is_protected() {
        let row = provenance(2, 1, ParentContext::NoParent, None, Some(499));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(result.state, EligibilityState::Protected);
        assert_eq!(result.reasons, vec![EligibilityReason::SystemOwner]);
    }

    #[test]
    fn user_owned_process_is_not_protected_by_ownership() {
        let row = provenance(2, 1, ParentContext::NoParent, None, Some(501));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(result.state, EligibilityState::Eligible);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn active_terminal_chain_is_protected() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), Some("devserver"), Some(501)),
            provenance(2, 1, resolved(3, 1), Some("zsh"), Some(501)),
            provenance(3, 1, resolved(4, 1), Some("kitty"), Some(501)),
            provenance(4, 1, ParentContext::NoParent, Some("launchd"), Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Protected);
        assert!(result.reasons.contains(&EligibilityReason::ActiveTerminal));
        assert_eq!(
            result.terminal,
            TerminalAssociation::Observed {
                identity: (2, 1),
                program: "zsh".into()
            }
        );
    }

    #[test]
    fn terminal_match_is_case_insensitive() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), None, Some(501)),
            provenance(2, 1, ParentContext::NoParent, Some("TeRmInAl"), Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert!(matches!(
            result.terminal,
            TerminalAssociation::Observed {
                identity: (2, 1),
                ..
            }
        ));
    }

    #[test]
    fn login_shell_with_leading_dash_is_terminal_evidence() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), None, Some(501)),
            provenance(2, 1, ParentContext::NoParent, Some("-zsh"), Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Protected);
        assert!(result.reasons.contains(&EligibilityReason::ActiveTerminal));
    }

    #[test]
    fn terminal_absence_is_not_detachment() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), None, Some(501)),
            provenance(2, 1, ParentContext::NoParent, Some("launchd"), Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.terminal, TerminalAssociation::NotObserved);
        assert_eq!(result.state, EligibilityState::Eligible);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn missing_ancestor_row_is_unknown() {
        let row = provenance(1, 1, resolved(2, 1), None, Some(501));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_eq!(
            result.ancestry.as_ref().unwrap().outcome,
            AncestryOutcome::MissingAncestor { identity: (2, 1) }
        );
        assert!(
            result
                .reasons
                .contains(&EligibilityReason::AncestryIncomplete)
        );
    }

    #[test]
    fn unvalidated_parent_is_unknown_and_the_pid_is_not_followed() {
        let rows = vec![
            provenance(1, 1, ParentContext::PidOnly(100), None, Some(501)),
            provenance(100, 2, ParentContext::NoParent, None, Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_eq!(
            result.ancestry.as_ref().unwrap().outcome,
            AncestryOutcome::UnvalidatedParent { pid: 100 }
        );
        assert_eq!(result.ancestry.unwrap().hops, 0);
    }

    #[test]
    fn ambiguous_reparenting_is_not_eligible() {
        let row = provenance(1, 1, ParentContext::PidOnly(1), None, Some(501));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_ne!(result.state, EligibilityState::Eligible);
    }

    #[test]
    fn ancestry_cycle_terminates_and_is_unknown() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), None, Some(501)),
            provenance(2, 1, resolved(1, 1), None, Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_eq!(
            result.ancestry.as_ref().unwrap().outcome,
            AncestryOutcome::Cycle { identity: (1, 1) }
        );
    }

    #[test]
    fn ancestry_depth_is_bounded() {
        let rows = (0..=MAX_ANCESTRY_DEPTH)
            .map(|i| provenance(i + 1, 1, resolved(i + 2, 1), None, Some(501)))
            .collect::<Vec<_>>();
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Unknown);
        assert_eq!(
            result.ancestry.as_ref().unwrap().outcome,
            AncestryOutcome::DepthExceeded
        );
        assert_eq!(result.ancestry.unwrap().hops, MAX_ANCESTRY_DEPTH);
    }

    #[test]
    fn complete_user_chain_reaching_root_is_eligible() {
        let row = provenance(1, 1, ParentContext::NoParent, None, Some(501));
        let result = assess(row.identity(), &scan(vec![row]), None);
        assert_eq!(
            result.ancestry.unwrap().outcome,
            AncestryOutcome::ReachedRoot
        );
        assert_eq!(result.state, EligibilityState::Eligible);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn parent_pid_one_reaching_root_is_eligible_not_abandoned() {
        let rows = vec![
            provenance(1, 1, resolved(2, 1), None, Some(501)),
            provenance(2, 1, ParentContext::NoParent, None, Some(501)),
        ];
        let result = assess((1, 1), &scan(rows), None);
        assert_eq!(result.state, EligibilityState::Eligible);
    }

    #[test]
    fn assessment_is_deterministic() {
        let row = provenance(1, 1, ParentContext::NoParent, None, None);
        let rows = scan(vec![row]);
        assert_eq!(assess((1, 1), &rows, None), assess((1, 1), &rows, None));
    }

    #[test]
    fn eligibility_states_are_exactly_three() {
        fn state_name(state: EligibilityState) -> &'static str {
            match state {
                EligibilityState::Protected => "protected",
                EligibilityState::Eligible => "eligible",
                EligibilityState::Unknown => "unknown",
            }
        }
        assert_eq!(state_name(EligibilityState::Protected), "protected");
        assert_eq!(state_name(EligibilityState::Eligible), "eligible");
        assert_eq!(state_name(EligibilityState::Unknown), "unknown");
    }

    #[test]
    fn monitor_identity_finds_the_current_process() {
        let pid = std::process::id();
        let row = provenance(pid, 1, ParentContext::NoParent, None, Some(501));
        assert_eq!(monitor_identity(&scan(vec![row])), Some((pid, 1)));
    }
}
