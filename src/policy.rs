use crate::topology::{ActiveMode, AltMode, PortSnapshot, UsbMode};
use crate::vdo::{self, DISPLAYPORT_SVID, THUNDERBOLT_SVID};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Candidate {
    Usb4,
    Thunderbolt { svid: u16, mode: u8 },
    DisplayPort { svid: u16, mode: u8 },
    OrdinaryUsb,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WaitReason {
    PartnerIdentity,
    PartnerModes,
    CableDiscovery,
    CableIdentity,
    Usb4Router,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Wait(WaitReason),
    Keep(ActiveMode),
    Transition { from: ActiveMode, to: Candidate },
    NoMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Context {
    pub discovery_expired: bool,
}

fn mode_candidate(mode: &AltMode) -> Candidate {
    if mode.svid == THUNDERBOLT_SVID {
        Candidate::Thunderbolt {
            svid: mode.svid,
            mode: mode.mode,
        }
    } else {
        Candidate::DisplayPort {
            svid: mode.svid,
            mode: mode.mode,
        }
    }
}

pub fn candidates(port: &PortSnapshot, context: Context) -> Result<Vec<Candidate>, WaitReason> {
    let Some(partner) = &port.partner else {
        return Ok(Vec::new());
    };
    if !partner.supports_pd {
        return Ok(vec![Candidate::OrdinaryUsb]);
    }
    let Some(identity) = &partner.identity else {
        return if context.discovery_expired {
            Ok(vec![Candidate::OrdinaryUsb])
        } else {
            Err(WaitReason::PartnerIdentity)
        };
    };
    if identity.id_header == 0 {
        return if context.discovery_expired {
            Ok(vec![Candidate::OrdinaryUsb])
        } else {
            Err(WaitReason::PartnerIdentity)
        };
    }

    let mut result = Vec::new();
    if port.supports_usb4
        && vdo::partner_supports_usb4(identity)
        && partner.usb_modes.contains(UsbMode::Usb4)
    {
        match &port.cable {
            Some(cable) => match &cable.identity {
                Some(cable_id) if vdo::cable_supports_usb4(cable_id, &cable.plug_modes) => {
                    result.push(Candidate::Usb4)
                }
                Some(_) => {}
                None if !context.discovery_expired => return Err(WaitReason::CableIdentity),
                None => {}
            },
            None if !context.discovery_expired => return Err(WaitReason::CableDiscovery),
            None => {}
        }
    }

    if port.supports_usb4 && vdo::modal(identity) && vdo::supports_tbt(&partner.alt_modes) {
        if let Some(cable) = &port.cable {
            if let Some(cable_id) = &cable.identity {
                if vdo::cable_supports_tbt(
                    cable_id,
                    &cable.plug_modes,
                    cable.pd_revision.as_deref(),
                ) && let Some(mode) = partner
                    .alt_modes
                    .iter()
                    .find(|m| m.svid == THUNDERBOLT_SVID)
                {
                    result.push(mode_candidate(mode));
                }
            } else if !context.discovery_expired {
                return Err(WaitReason::CableIdentity);
            }
        } else if !context.discovery_expired {
            return Err(WaitReason::CableDiscovery);
        }
    }
    if vdo::supports_dp(&partner.alt_modes) {
        let cable_valid = !vdo::dp_partner_receptacle(&partner.alt_modes)
            || port
                .cable
                .as_ref()
                .and_then(|c| {
                    c.identity
                        .as_ref()
                        .map(|id| vdo::cable_supports_dp(id, &c.plug_modes))
                })
                .unwrap_or(true);
        if cable_valid
            && let Some(mode) = partner
                .alt_modes
                .iter()
                .find(|m| m.svid == DISPLAYPORT_SVID)
        {
            result.push(mode_candidate(mode));
        }
    }
    if result.is_empty() {
        result.push(Candidate::OrdinaryUsb);
    }
    Ok(result)
}

fn active_matches(active: &ActiveMode, candidate: &Candidate) -> bool {
    matches!(
        (active, candidate),
        (ActiveMode::Usb4, Candidate::Usb4)
            | (
                ActiveMode::Thunderbolt { .. },
                Candidate::Thunderbolt { .. }
            )
            | (
                ActiveMode::DisplayPort { .. },
                Candidate::DisplayPort { .. }
            )
            | (ActiveMode::None, Candidate::OrdinaryUsb)
    )
}

pub fn decide(port: &PortSnapshot, context: Context) -> Decision {
    let candidates = match candidates(port, context) {
        Ok(value) => value,
        Err(reason) => return Decision::Wait(reason),
    };
    let Some(best) = candidates.first() else {
        return if port.active_mode == ActiveMode::None {
            Decision::NoMode
        } else {
            Decision::Transition {
                from: port.active_mode.clone(),
                to: Candidate::OrdinaryUsb,
            }
        };
    };
    if active_matches(&port.active_mode, best) {
        Decision::Keep(port.active_mode.clone())
    } else {
        Decision::Transition {
            from: port.active_mode.clone(),
            to: best.clone(),
        }
    }
}
