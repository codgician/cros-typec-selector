//! Standards-derived checks ported from ChromeOS typecd's
//! `Port::CanEnterUSB4`, `Port::CanEnterTBTCompatibilityMode`,
//! `Port::CanEnterDPAltMode`, and `Cable::*PDIdentityCheck`.
//! Bit definitions are cross-checked with Linux `pd_vdo.h` and `typec_tbt.h`.

use crate::topology::{AltMode, PdIdentity};

pub const DISPLAYPORT_SVID: u16 = 0xff01;
pub const THUNDERBOLT_SVID: u16 = 0x8087;

const IDH_PRODUCT_TYPE_SHIFT: u8 = 27;
const IDH_PRODUCT_TYPE_MASK: u32 = 0x7;
const IDH_MODAL_OPERATION: u32 = 1 << 26;
const UFP_DEVICE_CAPABILITY_SHIFT: u8 = 24;
const UFP_DEVICE_CAPABILITY_MASK: u32 = 0xf;
const UFP_USB4_CAPABLE: u32 = 0x8;
const CABLE_SPEED_MASK: u32 = 0x7;
const ACTIVE_CABLE_VDO_VERSION_SHIFT: u8 = 21;
const ACTIVE_CABLE_VDO_VERSION_MASK: u32 = 0x7;
const ACTIVE_CABLE_VDO_VERSION_1_3: u32 = 0x3;
const ACTIVE_CABLE_USB4_NOT_SUPPORTED: u32 = 1 << 8;
const TBT_CABLE_SPEED_SHIFT: u8 = 16;
const TBT_CABLE_SPEED_MASK: u32 = 0x7;
const TBT_CABLE_10_AND_20_GBPS: u32 = 0x3;
const TBT_ROUNDED_SHIFT: u8 = 19;
const TBT_ROUNDED_MASK: u32 = 0x3;
const TBT_GEN3_GEN4_ROUNDED: u32 = 0x1;
const DP_MODE_RECEPTACLE: u32 = 0x40;
const DP_MODE_SINK: u32 = 0x1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductType {
    Undefined,
    Hub,
    Peripheral,
    PassiveCable,
    ActiveCable,
    Ama,
    Vpd,
    Other(u8),
}

pub fn product_type(identity: &PdIdentity) -> ProductType {
    match ((identity.id_header >> IDH_PRODUCT_TYPE_SHIFT) & IDH_PRODUCT_TYPE_MASK) as u8 {
        0 => ProductType::Undefined,
        1 => ProductType::Hub,
        2 => ProductType::Peripheral,
        3 => ProductType::PassiveCable,
        4 => ProductType::ActiveCable,
        5 => ProductType::Ama,
        6 => ProductType::Vpd,
        value => ProductType::Other(value),
    }
}

pub fn modal(identity: &PdIdentity) -> bool {
    identity.id_header & IDH_MODAL_OPERATION != 0
}

pub fn partner_supports_usb4(identity: &PdIdentity) -> bool {
    matches!(
        product_type(identity),
        ProductType::Hub | ProductType::Peripheral
    ) && identity.product_type_vdos[0].is_some_and(|vdo| {
        ((vdo >> UFP_DEVICE_CAPABILITY_SHIFT) & UFP_DEVICE_CAPABILITY_MASK) & UFP_USB4_CAPABLE != 0
    })
}

pub fn cable_supports_usb4(identity: &PdIdentity, modes: &[AltMode]) -> bool {
    let Some(vdo1) = identity.product_type_vdos[0] else {
        return false;
    };
    match product_type(identity) {
        ProductType::PassiveCable => vdo1 & CABLE_SPEED_MASK != 0,
        ProductType::ActiveCable => {
            let version = (vdo1 >> ACTIVE_CABLE_VDO_VERSION_SHIFT) & ACTIVE_CABLE_VDO_VERSION_MASK;
            if version == ACTIVE_CABLE_VDO_VERSION_1_3 {
                return identity.product_type_vdos[1]
                    .is_some_and(|vdo2| vdo2 & ACTIVE_CABLE_USB4_NOT_SUPPORTED == 0);
            }
            modal(identity)
                && modes
                    .iter()
                    .filter(|m| m.svid == THUNDERBOLT_SVID)
                    .any(|m| {
                        let speed = (m.vdo >> TBT_CABLE_SPEED_SHIFT) & TBT_CABLE_SPEED_MASK;
                        let rounded = (m.vdo >> TBT_ROUNDED_SHIFT) & TBT_ROUNDED_MASK;
                        speed == TBT_CABLE_10_AND_20_GBPS && rounded == TBT_GEN3_GEN4_ROUNDED
                    })
        }
        _ => false,
    }
}

pub fn cable_supports_tbt(
    identity: &PdIdentity,
    modes: &[AltMode],
    pd_revision: Option<&str>,
) -> bool {
    match product_type(identity) {
        ProductType::ActiveCable => modes.iter().any(|m| m.svid == THUNDERBOLT_SVID),
        ProductType::PassiveCable => {
            let speed = identity.product_type_vdos[0].unwrap_or(0) & CABLE_SPEED_MASK;
            match pd_revision {
                Some(rev) if rev.starts_with("2.") => matches!(speed, 1 | 2),
                Some(_) => matches!(speed, 1..=4),
                None => false,
            }
        }
        _ => false,
    }
}

pub fn dp_partner_receptacle(modes: &[AltMode]) -> bool {
    modes
        .iter()
        .any(|m| m.svid == DISPLAYPORT_SVID && m.vdo & DP_MODE_RECEPTACLE != 0)
}

pub fn cable_supports_dp(identity: &PdIdentity, modes: &[AltMode]) -> bool {
    let Some(vdo1) = identity.product_type_vdos[0] else {
        return false;
    };
    match product_type(identity) {
        ProductType::PassiveCable => vdo1 & CABLE_SPEED_MASK != 0,
        ProductType::ActiveCable => {
            modal(identity) && modes.iter().any(|m| m.svid == DISPLAYPORT_SVID)
        }
        _ => false,
    }
}

pub fn supports_dp(modes: &[AltMode]) -> bool {
    modes
        .iter()
        .any(|m| m.svid == DISPLAYPORT_SVID && m.vdo & DP_MODE_SINK != 0)
}
pub fn supports_tbt(modes: &[AltMode]) -> bool {
    modes.iter().any(|m| m.svid == THUNDERBOLT_SVID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn identity(idh: u32, vdo1: Option<u32>, vdo2: Option<u32>) -> PdIdentity {
        PdIdentity {
            id_header: idh,
            cert_stat: 0,
            product: 0,
            product_type_vdos: [vdo1, vdo2, None],
        }
    }
    #[test]
    fn partner_usb4_bit_boundary() {
        let id = identity(1 << 27, Some(0x0800_0000), None);
        assert!(partner_supports_usb4(&id));
        let id = identity(1 << 27, Some(0x0400_0000), None);
        assert!(!partner_supports_usb4(&id));
    }
    #[test]
    fn passive_usb2_is_not_usb4() {
        assert!(!cable_supports_usb4(&identity(3 << 27, Some(0), None), &[]));
        assert!(cable_supports_usb4(&identity(3 << 27, Some(1), None), &[]));
    }
    #[test]
    fn active_v13_inverted_usb4_bit() {
        assert!(cable_supports_usb4(
            &identity(4 << 27, Some(3 << 21), Some(0)),
            &[]
        ));
        assert!(!cable_supports_usb4(
            &identity(4 << 27, Some(3 << 21), Some(1 << 8)),
            &[]
        ));
    }
    #[test]
    fn legacy_active_requires_rounded_tbt() {
        let mode = AltMode {
            syspath: PathBuf::new(),
            svid: THUNDERBOLT_SVID,
            mode: 1,
            vdo: (3 << 16) | (1 << 19),
            active: false,
        };
        assert!(cable_supports_usb4(
            &identity((4 << 27) | (1 << 26), Some(0), None),
            &[mode]
        ));
    }
    #[test]
    fn passive_usb2_is_not_dp_capable() {
        assert!(!cable_supports_dp(&identity(3 << 27, Some(0), None), &[]));
        assert!(cable_supports_dp(&identity(3 << 27, Some(1), None), &[]));
    }
}
