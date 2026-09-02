use crate::error::{Error, Result};
use crate::policy::{self, Candidate, Context, Decision};
use crate::state::{PortState, RETRY_DELAYS, TRANSITION_TIMEOUT};
use crate::sysfs::{
    Operation, Sysfs, SysfsControl, TypecControl, downstream_router_present_in, usb4_router_present,
};
use crate::topology::{ActiveMode, PortSnapshot};
use crate::udev::{EventHint, Monitor};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    DryRun,
    Live,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStage {
    Exit,
    Enter,
    FallbackExit,
}

#[derive(Debug)]
pub enum ReconcileEvent {
    Snapshot {
        port: String,
        generation: u64,
        candidates: std::result::Result<Vec<Candidate>, crate::policy::WaitReason>,
        decision: Decision,
    },
    Detached {
        port: String,
        generation: u64,
    },
    FailedTerminal {
        port: String,
        generation: u64,
        active: ActiveMode,
        reason: String,
    },
    CoordinationRequired {
        port: String,
        keeping: ActiveMode,
    },
    Operation {
        port: String,
        stage: OperationStage,
        attempt: Option<usize>,
        operation: Operation,
    },
    WaitForNone {
        port: String,
    },
    RequestError {
        port: String,
        attempt: usize,
        error: String,
    },
    CompletionTimeout {
        port: String,
        target: Candidate,
    },
    CandidateFailed {
        port: String,
        target: Candidate,
    },
    Active {
        port: String,
        target: Candidate,
    },
    OrdinaryUsb {
        port: String,
    },
    Cancelled {
        port: String,
        stage: &'static str,
    },
    ExitTimeout {
        port: String,
    },
    FailedCandidates {
        port: String,
    },
    TopologyError {
        port: String,
        error: String,
    },
    ReconcileError {
        port: String,
        error: String,
    },
}

impl fmt::Display for ReconcileEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

struct TransitionReport {
    events: Vec<ReconcileEvent>,
    failed: bool,
}

impl TransitionReport {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            failed: false,
        }
    }
}

pub struct Daemon {
    sysfs: Sysfs,
    mode: Mode,
    states: HashMap<String, PortState>,
    monitor: Option<Monitor>,
}

impl Daemon {
    pub fn new(sysfs: Sysfs, mode: Mode) -> Self {
        Self {
            sysfs,
            mode,
            states: HashMap::new(),
            monitor: None,
        }
    }

    pub fn reconcile_all(&mut self) -> Result<Vec<ReconcileEvent>> {
        let names = self.sysfs.port_names()?;
        let present = names.iter().cloned().collect::<HashSet<_>>();
        for (name, state) in &mut self.states {
            if !present.contains(name) && !matches!(state.phase, crate::state::Phase::Detached) {
                state.detach();
            }
        }
        let mut events = Vec::new();
        for name in names {
            match self.sysfs.port(&name) {
                Ok(port) => match self.reconcile_snapshot(port) {
                    Ok(port_events) => events.extend(port_events),
                    Err(error) => events.push(ReconcileEvent::ReconcileError {
                        port: name,
                        error: error.to_string(),
                    }),
                },
                Err(error) => events.push(ReconcileEvent::TopologyError {
                    port: name,
                    error: error.to_string(),
                }),
            }
        }
        Ok(events)
    }

    pub fn reconcile_port(&mut self, name: &str) -> Result<Vec<ReconcileEvent>> {
        let port = self.sysfs.port(name)?;
        self.reconcile_snapshot(port)
    }

    fn reconcile_snapshot(&mut self, port: PortSnapshot) -> Result<Vec<ReconcileEvent>> {
        let now = Instant::now();
        let state = self.states.entry(port.name.clone()).or_default();
        if port.partner.is_some() && matches!(state.phase, crate::state::Phase::Detached) {
            state.attach(now);
        }
        if port.partner.is_none() {
            if !matches!(state.phase, crate::state::Phase::Detached) {
                state.detach();
            }
            return Ok(vec![ReconcileEvent::Detached {
                port: port.name,
                generation: state.generation,
            }]);
        }
        if let crate::state::Phase::Failed(reason) = &state.phase {
            return Ok(vec![ReconcileEvent::FailedTerminal {
                port: port.name,
                generation: state.generation,
                active: port.active_mode,
                reason: reason.clone(),
            }]);
        }
        let expired = state.deadline.is_some_and(|deadline| now >= deadline);
        let context = Context {
            discovery_expired: expired,
        };
        let ordered = policy::candidates(&port, context);
        let mut decision = policy::decide(&port, context);
        if matches!(decision, Decision::Keep(ActiveMode::Usb4))
            && !port.usb4_link.as_ref().is_some_and(usb4_router_present)
        {
            decision = if expired {
                ordered
                    .as_ref()
                    .ok()
                    .and_then(|values| values.first())
                    .cloned()
                    .map_or(Decision::NoMode, |to| Decision::Transition {
                        from: ActiveMode::Usb4,
                        to,
                    })
            } else {
                Decision::Wait(crate::policy::WaitReason::Usb4Router)
            };
        }
        state.apply_decision(&decision);
        let generation = state.generation;
        let mut events = vec![ReconcileEvent::Snapshot {
            port: port.name.clone(),
            generation,
            candidates: ordered.clone(),
            decision: decision.clone(),
        }];
        if let Decision::Transition { from, .. } = decision {
            if requires_coordination(self.mode, &from) {
                self.states
                    .get_mut(&port.name)
                    .expect("port state exists")
                    .phase = crate::state::Phase::Active(from.clone());
                events.push(ReconcileEvent::CoordinationRequired {
                    port: port.name,
                    keeping: from,
                });
                return Ok(events);
            }
            let targets = ordered.unwrap_or_default();
            let report = self.transition(&port, generation, from, targets)?;
            if report.failed && self.valid(&port.name, generation) {
                self.states
                    .get_mut(&port.name)
                    .expect("port state exists")
                    .phase = crate::state::Phase::Failed("bounded transition failure".into());
            }
            events.extend(report.events);
        }
        Ok(events)
    }

    fn valid(&mut self, port: &str, generation: u64) -> bool {
        self.drain_monitor();
        self.states
            .get(port)
            .is_some_and(|state| state.generation_is(generation))
    }

    fn drain_monitor(&mut self) -> bool {
        let events = self
            .monitor
            .as_ref()
            .map(Monitor::drain)
            .unwrap_or_default();
        let had_events = !events.is_empty();
        for event in events {
            self.event(&event);
        }
        had_events
    }

    fn transition(
        &mut self,
        initial: &PortSnapshot,
        generation: u64,
        from: ActiveMode,
        targets: Vec<Candidate>,
    ) -> Result<TransitionReport> {
        let mut report = TransitionReport::new();
        let mut control = SysfsControl::new(self.mode == Mode::DryRun);
        let usb4_domain = initial
            .usb4_link
            .as_ref()
            .and_then(|link| link.domain_syspath.clone());
        if from != ActiveMode::None {
            if !self.valid(&initial.name, generation) {
                report.events.push(ReconcileEvent::Cancelled {
                    port: initial.name.clone(),
                    stage: "before-exit",
                });
                return Ok(report);
            }
            let operation = control.exit(initial)?;
            report.events.push(ReconcileEvent::Operation {
                port: initial.name.clone(),
                stage: OperationStage::Exit,
                attempt: None,
                operation,
            });
            if self.mode == Mode::DryRun {
                report.events.push(ReconcileEvent::WaitForNone {
                    port: initial.name.clone(),
                });
            } else if !self.wait_for(
                initial,
                generation,
                |port| port.active_mode == ActiveMode::None,
                Duration::from_secs(5),
            )? {
                if self.valid(&initial.name, generation) {
                    report.failed = true;
                    report.events.push(ReconcileEvent::ExitTimeout {
                        port: initial.name.clone(),
                    });
                } else {
                    report.events.push(ReconcileEvent::Cancelled {
                        port: initial.name.clone(),
                        stage: "exit",
                    });
                }
                return Ok(report);
            }
        }
        for target in targets {
            if target == Candidate::OrdinaryUsb {
                report.events.push(ReconcileEvent::OrdinaryUsb {
                    port: initial.name.clone(),
                });
                return Ok(report);
            }
            if self.mode == Mode::DryRun {
                let fresh = self
                    .sysfs
                    .port(&initial.name)
                    .unwrap_or_else(|_| initial.clone());
                let operation = control.enter(&fresh, &target)?;
                report.events.push(ReconcileEvent::Operation {
                    port: initial.name.clone(),
                    stage: OperationStage::Enter,
                    attempt: None,
                    operation,
                });
                return Ok(report);
            }
            for (attempt, delay) in RETRY_DELAYS.iter().enumerate() {
                let attempt = attempt + 1;
                if !self.valid(&initial.name, generation) {
                    report.events.push(ReconcileEvent::Cancelled {
                        port: initial.name.clone(),
                        stage: "before-enter",
                    });
                    return Ok(report);
                }
                let fresh = self.sysfs.port(&initial.name)?;
                match control.enter(&fresh, &target) {
                    Ok(operation) => {
                        report.events.push(ReconcileEvent::Operation {
                            port: initial.name.clone(),
                            stage: OperationStage::Enter,
                            attempt: Some(attempt),
                            operation,
                        });
                        if self.wait_for(
                            initial,
                            generation,
                            |port| target_active(port, &target, usb4_domain.as_deref()),
                            TRANSITION_TIMEOUT,
                        )? {
                            report.events.push(ReconcileEvent::Active {
                                port: initial.name.clone(),
                                target,
                            });
                            return Ok(report);
                        }
                        report.events.push(ReconcileEvent::CompletionTimeout {
                            port: initial.name.clone(),
                            target: target.clone(),
                        });
                        break;
                    }
                    Err(error) => report.events.push(ReconcileEvent::RequestError {
                        port: initial.name.clone(),
                        attempt,
                        error: error.to_string(),
                    }),
                }
                self.wait_for_event_or_timeout(*delay)?;
            }
            report.failed = true;
            report.events.push(ReconcileEvent::CandidateFailed {
                port: initial.name.clone(),
                target: target.clone(),
            });
            let fresh = match self.sysfs.port(&initial.name) {
                Ok(port) => port,
                Err(_) => return Ok(report),
            };
            if fresh.active_mode != ActiveMode::None {
                if !self.valid(&initial.name, generation) {
                    report.events.push(ReconcileEvent::Cancelled {
                        port: initial.name.clone(),
                        stage: "before-fallback-exit",
                    });
                    return Ok(report);
                }
                let operation = control.exit(&fresh)?;
                report.events.push(ReconcileEvent::Operation {
                    port: initial.name.clone(),
                    stage: OperationStage::FallbackExit,
                    attempt: None,
                    operation,
                });
                if !self.wait_for(
                    initial,
                    generation,
                    |port| port.active_mode == ActiveMode::None,
                    Duration::from_secs(5),
                )? {
                    return Ok(report);
                }
            }
        }
        report.failed = true;
        report.events.push(ReconcileEvent::FailedCandidates {
            port: initial.name.clone(),
        });
        Ok(report)
    }

    fn wait_for(
        &mut self,
        initial: &PortSnapshot,
        generation: u64,
        predicate: impl Fn(&PortSnapshot) -> bool,
        timeout: Duration,
    ) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !self.valid(&initial.name, generation) {
                return Ok(false);
            }
            match self.sysfs.port(&initial.name) {
                Ok(port) if predicate(&port) => return Ok(true),
                Ok(_) => {}
                Err(Error::InvalidPort(_)) => return Ok(false),
                Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            self.wait_for_event_or_timeout(remaining.min(Duration::from_millis(100)))?;
        }
        Ok(false)
    }

    fn wait_for_event_or_timeout(&mut self, timeout: Duration) -> Result<()> {
        if let Some(monitor) = &self.monitor {
            if monitor.wait(Some(timeout))? {
                self.drain_monitor();
            }
        } else {
            thread::sleep(timeout);
        }
        Ok(())
    }

    pub fn event(&mut self, event: &EventHint) {
        if event.partner
            && matches!(
                event.action,
                udev::EventType::Remove | udev::EventType::Unbind
            )
            && let Some(port) = &event.port
        {
            let state = self.states.entry(port.clone()).or_default();
            if !matches!(state.phase, crate::state::Phase::Detached) {
                state.detach();
            }
        }
    }

    fn next_discovery_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        self.states
            .values()
            .filter(|state| matches!(state.phase, crate::state::Phase::Discovering { .. }))
            .filter_map(|state| state.deadline)
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
    }

    pub fn run(mut self) -> Result<()> {
        self.monitor = Some(Monitor::new()?);
        for event in self.reconcile_all()? {
            println!("{event}");
        }
        loop {
            let timeout = self.next_discovery_timeout();
            let ready = self
                .monitor
                .as_ref()
                .expect("monitor initialized")
                .wait(timeout)?;
            if ready {
                thread::sleep(Duration::from_millis(75));
                self.drain_monitor();
            }
            if ready || timeout.is_some() {
                for event in self.reconcile_all()? {
                    println!("{event}");
                }
            }
        }
    }
}

fn requires_coordination(mode: Mode, active: &ActiveMode) -> bool {
    mode == Mode::Live
        && matches!(
            active,
            ActiveMode::DisplayPort { .. }
                | ActiveMode::Thunderbolt { .. }
                | ActiveMode::Other { .. }
        )
}

fn target_active(
    port: &PortSnapshot,
    target: &Candidate,
    usb4_domain: Option<&std::path::Path>,
) -> bool {
    match target {
        Candidate::Usb4 => {
            port.active_mode == ActiveMode::Usb4
                && usb4_domain.is_some_and(downstream_router_present_in)
        }
        Candidate::Thunderbolt { .. } => matches!(port.active_mode, ActiveMode::Thunderbolt { .. }),
        Candidate::DisplayPort { .. } => matches!(port.active_mode, ActiveMode::DisplayPort { .. }),
        Candidate::OrdinaryUsb => port.active_mode == ActiveMode::None,
    }
}

pub fn operation_is_authorization(operation: &Operation) -> bool {
    let path = match operation {
        Operation::Exit { path }
        | Operation::EnterUsb4 { path }
        | Operation::EnterAltMode { path, .. } => path,
    };
    path.file_name().is_some_and(|name| name == "authorized")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn backend_operations_cannot_name_authorized() {
        let op = Operation::EnterUsb4 {
            path: PathBuf::from("usb_mode"),
        };
        assert!(!operation_is_authorization(&op));
    }
    #[test]
    fn live_mode_keeps_early_kernel_dp_entry() {
        assert!(requires_coordination(
            Mode::Live,
            &ActiveMode::DisplayPort {
                syspath: "dp".into()
            }
        ));
        assert!(!requires_coordination(
            Mode::DryRun,
            &ActiveMode::DisplayPort {
                syspath: "dp".into()
            }
        ));
    }
    #[test]
    fn finds_router_below_host_router() {
        let domain = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sysfs-usb4-location/domain9");
        assert!(downstream_router_present_in(&domain));
    }
}
