use cros_typec_selector::policy::{Candidate, Decision, WaitReason};
use cros_typec_selector::state::{Phase, PortState, TransitionStep, transition_steps};
use cros_typec_selector::topology::ActiveMode;
use std::time::Instant;

#[test]
fn incomplete_discovery_waits_and_detach_cancels_generation() {
    let mut state = PortState::default();
    state.attach(Instant::now());
    let generation = state.generation;
    state.apply_decision(&Decision::Wait(WaitReason::CableDiscovery));
    assert!(matches!(state.phase, Phase::Discovering { .. }));
    state.detach();
    assert!(!state.generation_is(generation));
}

#[test]
fn replacement_mode_has_safe_ordering() {
    assert_eq!(
        transition_steps(
            &ActiveMode::Thunderbolt {
                syspath: "tbt".into()
            },
            &Candidate::DisplayPort {
                svid: 0xff01,
                mode: 1
            }
        ),
        vec![
            TransitionStep::Exit,
            TransitionStep::WaitForNone,
            TransitionStep::Enter(Candidate::DisplayPort {
                svid: 0xff01,
                mode: 1
            })
        ]
    );
}

#[test]
fn retries_are_bounded() {
    assert_eq!(cros_typec_selector::state::RETRY_DELAYS.len(), 3);
}
