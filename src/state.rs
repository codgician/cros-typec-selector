use crate::policy::{Candidate, Decision, WaitReason};
use crate::topology::ActiveMode;
use std::time::{Duration, Instant};

pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
pub const TRANSITION_TIMEOUT: Duration = Duration::from_secs(5);
pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(300),
    Duration::from_millis(900),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Phase {
    Detached,
    Discovering { reason: WaitReason },
    Selecting,
    Exiting { target: Candidate },
    Entering { target: Candidate, attempt: u8 },
    Active(ActiveMode),
    Idle,
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionStep {
    Exit,
    WaitForNone,
    Enter(Candidate),
}

pub fn transition_steps(from: &ActiveMode, target: &Candidate) -> Vec<TransitionStep> {
    let already_active = matches!(
        (from, target),
        (ActiveMode::Usb4, Candidate::Usb4)
            | (
                ActiveMode::Thunderbolt { .. },
                Candidate::Thunderbolt { .. }
            )
            | (
                ActiveMode::DisplayPort { .. },
                Candidate::DisplayPort { .. }
            )
    );
    if already_active || (*from == ActiveMode::None && *target == Candidate::OrdinaryUsb) {
        Vec::new()
    } else if *from == ActiveMode::None {
        vec![TransitionStep::Enter(target.clone())]
    } else if *target == Candidate::OrdinaryUsb {
        vec![TransitionStep::Exit, TransitionStep::WaitForNone]
    } else {
        vec![
            TransitionStep::Exit,
            TransitionStep::WaitForNone,
            TransitionStep::Enter(target.clone()),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct PortState {
    pub generation: u64,
    pub phase: Phase,
    pub attached_at: Option<Instant>,
    pub deadline: Option<Instant>,
}

impl Default for PortState {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: Phase::Detached,
            attached_at: None,
            deadline: None,
        }
    }
}

impl PortState {
    pub fn attach(&mut self, now: Instant) {
        self.generation = self.generation.wrapping_add(1);
        self.phase = Phase::Discovering {
            reason: WaitReason::PartnerIdentity,
        };
        self.attached_at = Some(now);
        self.deadline = Some(now + DISCOVERY_TIMEOUT);
    }
    pub fn detach(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.phase = Phase::Detached;
        self.attached_at = None;
        self.deadline = None;
    }
    pub fn apply_decision(&mut self, decision: &Decision) {
        self.phase = match decision {
            Decision::Wait(reason) => Phase::Discovering {
                reason: reason.clone(),
            },
            Decision::Keep(mode) => Phase::Active(mode.clone()),
            Decision::Transition { from, to } if *from == ActiveMode::None => Phase::Entering {
                target: to.clone(),
                attempt: 0,
            },
            Decision::Transition { to, .. } => Phase::Exiting { target: to.clone() },
            Decision::NoMode => Phase::Idle,
        };
    }
    pub fn generation_is(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detach_invalidates_generation() {
        let now = Instant::now();
        let mut state = PortState::default();
        state.attach(now);
        let old = state.generation;
        state.detach();
        assert!(!state.generation_is(old));
        assert_eq!(state.phase, Phase::Detached);
    }
    #[test]
    fn duplicate_keep_is_terminal() {
        let mut state = PortState::default();
        state.apply_decision(&Decision::Keep(ActiveMode::Usb4));
        let first = state.phase.clone();
        state.apply_decision(&Decision::Keep(ActiveMode::Usb4));
        assert_eq!(state.phase, first);
    }
    #[test]
    fn dp_to_usb4_orders_exit_wait_enter() {
        assert_eq!(
            transition_steps(
                &ActiveMode::DisplayPort {
                    syspath: "dp".into()
                },
                &Candidate::Usb4
            ),
            vec![
                TransitionStep::Exit,
                TransitionStep::WaitForNone,
                TransitionStep::Enter(Candidate::Usb4)
            ]
        );
    }
    #[test]
    fn already_correct_mode_needs_no_steps() {
        assert!(transition_steps(&ActiveMode::Usb4, &Candidate::Usb4).is_empty());
    }
}
