use cros_typec_selector::policy::{self, Candidate, Context, Decision};
use cros_typec_selector::topology::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn number(value: &str) -> u32 {
    value.strip_prefix("0x").map_or_else(
        || value.parse().unwrap(),
        |v| u32::from_str_radix(v, 16).unwrap(),
    )
}
fn modes(value: Option<&&str>, cable: bool) -> Vec<AltMode> {
    value
        .into_iter()
        .flat_map(|v| v.split(','))
        .filter(|v| !v.is_empty())
        .map(|item| {
            let mut fields = item.split(':');
            AltMode {
                syspath: PathBuf::from(if cable { "plug-mode" } else { "partner-mode" }),
                svid: u16::from_str_radix(fields.next().unwrap(), 16).unwrap(),
                mode: fields.next().unwrap().parse().unwrap(),
                vdo: number(fields.next().unwrap()),
                active: false,
            }
        })
        .collect()
}
fn fixture(path: &Path) -> (PortSnapshot, Context, String) {
    let input = fs::read_to_string(path.join("topology.fixture")).unwrap();
    let map = input
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| line.split_once('=').unwrap())
        .collect::<HashMap<_, _>>();
    let identity = map.get("partner_idh").map(|idh| PdIdentity {
        id_header: number(idh),
        cert_stat: 0,
        product: 0,
        product_type_vdos: [map.get("partner_vdo1").map(|v| number(v)), None, None],
    });
    let partner_modes = modes(map.get("partner_modes"), false);
    let usb = map
        .get("usb_modes")
        .into_iter()
        .flat_map(|v| v.split(','))
        .map(|v| match v {
            "usb2" => UsbMode::Usb2,
            "usb3" => UsbMode::Usb3,
            "usb4" => UsbMode::Usb4,
            _ => panic!("bad USB mode"),
        })
        .collect();
    let partner = PartnerSnapshot {
        syspath: "partner".into(),
        supports_pd: map.get("partner_pd") == Some(&"true"),
        pd_revision: Some("3.0".into()),
        identity,
        usb_modes: UsbModeSet {
            available: usb,
            active: Some(UsbMode::Usb2),
        },
        alt_modes: partner_modes,
    };
    let cable = map.get("cable_idh").map(|idh| CableSnapshot {
        syspath: "cable".into(),
        pd_revision: map
            .get("cable_revision")
            .map(|v| (*v).into())
            .or(Some("3.0".into())),
        identity: Some(PdIdentity {
            id_header: number(idh),
            cert_stat: 0,
            product: 0,
            product_type_vdos: [
                map.get("cable_vdo1").map(|v| number(v)),
                map.get("cable_vdo2").map(|v| number(v)),
                None,
            ],
        }),
        plug_modes: modes(map.get("cable_modes"), true),
    });
    let port = PortSnapshot {
        syspath: "port".into(),
        name: "fixture-port".into(),
        data_role: Some(DataRole::Host),
        supports_usb4: map.get("port_usb4") == Some(&"true"),
        partner: Some(partner),
        cable,
        active_mode: ActiveMode::None,
        usb4_link: None,
    };
    (
        port,
        Context {
            discovery_expired: map.get("discovery_expired") == Some(&"true"),
        },
        map["expected"].into(),
    )
}

#[test]
fn capability_matrix() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in [
        "hp-g6-usb4",
        "apple-studio-display-usb4",
        "passive-usb4",
        "active-usb4",
        "tbt3-only",
        "displayport-only",
        "usb3-only",
        "power-only",
        "missing-emarker",
        "incomplete-discovery",
    ] {
        let (port, context, expected) = fixture(&root.join(name));
        let decision = policy::decide(&port, context);
        let actual = match decision {
            Decision::Wait(_) => "wait",
            Decision::Transition {
                to: Candidate::Usb4,
                ..
            } => "usb4",
            Decision::Transition {
                to: Candidate::Thunderbolt { .. },
                ..
            } => "tbt",
            Decision::Transition {
                to: Candidate::DisplayPort { .. },
                ..
            } => "dp",
            Decision::Transition {
                to: Candidate::OrdinaryUsb,
                ..
            }
            | Decision::Keep(ActiveMode::None) => "ordinary",
            other => panic!("unexpected decision: {other:?}"),
        };
        println!("scenario={name} expected={expected} observed={actual}");
        assert_eq!(actual, expected, "fixture {name}");
    }
}

#[test]
fn active_usb4_is_idempotent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/passive-usb4");
    let (mut port, context, _) = fixture(&root);
    port.active_mode = ActiveMode::Usb4;
    assert_eq!(
        policy::decide(&port, context),
        Decision::Keep(ActiveMode::Usb4)
    );
}
