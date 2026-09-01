use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRole {
    Host,
    Device,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbMode {
    Usb2,
    Usb3,
    Usb4,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsbModeSet {
    pub available: Vec<UsbMode>,
    pub active: Option<UsbMode>,
}

impl UsbModeSet {
    pub fn contains(&self, mode: UsbMode) -> bool {
        self.available.contains(&mode)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdIdentity {
    pub id_header: u32,
    pub cert_stat: u32,
    pub product: u32,
    pub product_type_vdos: [Option<u32>; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AltMode {
    pub syspath: PathBuf,
    pub svid: u16,
    pub mode: u8,
    pub vdo: u32,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartnerSnapshot {
    pub syspath: PathBuf,
    pub supports_pd: bool,
    pub pd_revision: Option<String>,
    pub identity: Option<PdIdentity>,
    pub usb_modes: UsbModeSet,
    pub alt_modes: Vec<AltMode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CableSnapshot {
    pub syspath: PathBuf,
    pub pd_revision: Option<String>,
    pub identity: Option<PdIdentity>,
    pub plug_modes: Vec<AltMode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActiveMode {
    None,
    Usb4,
    Thunderbolt { syspath: PathBuf },
    DisplayPort { syspath: PathBuf },
    Other { svid: u16, syspath: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Usb4Link {
    pub syspath: PathBuf,
    pub domain_syspath: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortSnapshot {
    pub syspath: PathBuf,
    pub name: String,
    pub data_role: Option<DataRole>,
    pub supports_usb4: bool,
    pub partner: Option<PartnerSnapshot>,
    pub cable: Option<CableSnapshot>,
    pub active_mode: ActiveMode,
    pub usb4_link: Option<Usb4Link>,
}

impl fmt::Display for PortSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "port={} role={:?} host_usb4={} active={:?}",
            self.name, self.data_role, self.supports_usb4, self.active_mode
        )?;
        match &self.partner {
            None => writeln!(f, "  partner=none")?,
            Some(p) => {
                writeln!(
                    f,
                    "  partner=pd:{} revision:{:?} identity:{} usb:{:?}",
                    p.supports_pd,
                    p.pd_revision,
                    p.identity.is_some(),
                    p.usb_modes
                )?;
                for mode in &p.alt_modes {
                    writeln!(
                        f,
                        "  partner-mode=svid:{:04x} mode:{} active:{} vdo:{:#010x}",
                        mode.svid, mode.mode, mode.active, mode.vdo
                    )?;
                }
            }
        }
        match &self.cable {
            None => writeln!(f, "  cable=none")?,
            Some(c) => {
                writeln!(
                    f,
                    "  cable=identity:{} revision:{:?}",
                    c.identity.is_some(),
                    c.pd_revision
                )?;
                for mode in &c.plug_modes {
                    writeln!(
                        f,
                        "  cable-mode=svid:{:04x} mode:{} active:{} vdo:{:#010x}",
                        mode.svid, mode.mode, mode.active, mode.vdo
                    )?;
                }
            }
        }
        if let Some(link) = &self.usb4_link {
            writeln!(
                f,
                "  usb4-link={} domain={}",
                link.syspath.display(),
                link.domain_syspath
                    .as_ref()
                    .map_or("unknown".into(), |p| p.display().to_string())
            )?;
        }
        Ok(())
    }
}
