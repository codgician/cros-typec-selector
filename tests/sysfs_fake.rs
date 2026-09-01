use cros_typec_selector::error::Error;
use cros_typec_selector::sysfs::Sysfs;
use cros_typec_selector::topology::{DataRole, UsbMode};
use std::path::Path;

fn fixture(name: &str) -> Sysfs {
    Sysfs::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
}

#[test]
fn reads_partial_non_pd_topology_without_hardware() {
    let ports = fixture("sysfs-basic").ports().unwrap();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].data_role, Some(DataRole::Host));
    let partner = ports[0].partner.as_ref().unwrap();
    assert!(!partner.supports_pd);
    assert_eq!(partner.usb_modes.active, Some(UsbMode::Usb2));
    assert!(ports[0].cable.is_none());
}

#[test]
fn missing_partner_is_a_valid_detached_or_partial_snapshot() {
    let port = fixture("sysfs-partial").port("port3").unwrap();
    assert!(port.partner.is_none());
}

#[test]
fn malformed_vdo_fails_with_context() {
    let error = fixture("sysfs-malformed").port("port2").unwrap_err();
    assert!(matches!(error, Error::Parse { .. }));
    assert!(error.to_string().contains("id_header"));
}

#[test]
fn port_names_cannot_escape_the_sysfs_root() {
    assert!(matches!(
        fixture("sysfs-basic").port("../port0"),
        Err(Error::InvalidPort(_))
    ));
}

#[test]
fn associates_usb4_by_physical_location_not_numbers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-usb4-location");
    let port = Sysfs::new(root.join("typec"))
        .with_thunderbolt_root(root.join("domain9"))
        .port("port7")
        .unwrap();
    assert!(port.supports_usb4);
    assert!(
        port.usb4_link
            .unwrap()
            .domain_syspath
            .unwrap()
            .ends_with("domain9")
    );
}
