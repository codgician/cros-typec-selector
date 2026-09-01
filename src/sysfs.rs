use crate::error::{Error, Result};
use crate::policy::Candidate;
use crate::topology::*;
use crate::vdo::{DISPLAYPORT_SVID, THUNDERBOLT_SVID};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Sysfs {
    root: PathBuf,
    thunderbolt_root: Option<PathBuf>,
}

impl Default for Sysfs {
    fn default() -> Self {
        Self::new("/sys/class/typec")
    }
}

impl Sysfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let thunderbolt_root = (root == Path::new("/sys/class/typec"))
            .then(|| PathBuf::from("/sys/bus/thunderbolt/devices"));
        Self {
            root,
            thunderbolt_root,
        }
    }
    pub fn with_thunderbolt_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.thunderbolt_root = Some(root.into());
        self
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn port_names(&self) -> Result<Vec<String>> {
        let mut names = fs::read_dir(&self.root)
            .map_err(|e| Error::io(&self.root, e))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_port_name(name))
            .collect::<Vec<_>>();
        names.sort_by_key(|name| port_number(name).unwrap_or(u32::MAX));
        Ok(names)
    }

    pub fn ports(&self) -> Result<Vec<PortSnapshot>> {
        self.port_names()?
            .into_iter()
            .map(|name| self.port(&name))
            .collect()
    }

    pub fn port(&self, name: &str) -> Result<PortSnapshot> {
        if !is_port_name(name) {
            return Err(Error::InvalidPort(name.into()));
        }
        let syspath = self.root.join(name);
        if !syspath.exists() {
            return Err(Error::InvalidPort(name.into()));
        }
        let data_role = optional_text(&syspath.join("data_role"))?.map(|v| parse_role(&v));
        let partner_path = self.root.join(format!("{name}-partner"));
        let cable_path = self.root.join(format!("{name}-cable"));
        let partner = partner_path
            .exists()
            .then(|| self.partner(&partner_path))
            .transpose()?;
        let cable = cable_path
            .exists()
            .then(|| self.cable(name, &cable_path))
            .transpose()?;
        let (usb4_link, supports_usb4) =
            find_usb4_link(&syspath, self.thunderbolt_root.as_deref())?;
        let active_mode = active_mode(partner.as_ref());
        Ok(PortSnapshot {
            syspath,
            name: name.into(),
            data_role,
            supports_usb4,
            partner,
            cable,
            active_mode,
            usb4_link,
        })
    }

    fn partner(&self, path: &Path) -> Result<PartnerSnapshot> {
        let supports_pd =
            optional_text(&path.join("supports_usb_power_delivery"))?.as_deref() == Some("yes");
        let pd_revision = optional_text(&path.join("usb_power_delivery_revision"))?;
        let identity = read_identity(path)?;
        let usb_modes = optional_text(&path.join("usb_mode"))?
            .map_or_else(UsbModeSet::default, |v| parse_usb_modes(&v));
        let alt_modes = self.alt_modes(path)?;
        Ok(PartnerSnapshot {
            syspath: path.into(),
            supports_pd,
            pd_revision,
            identity,
            usb_modes,
            alt_modes,
        })
    }

    fn cable(&self, port: &str, path: &Path) -> Result<CableSnapshot> {
        let pd_revision = optional_text(&path.join("usb_power_delivery_revision"))?;
        let identity = read_identity(path)?;
        let plug = self.root.join(format!("{port}-plug0"));
        let plug_modes = if plug.exists() {
            self.alt_modes(&plug)?
        } else {
            Vec::new()
        };
        Ok(CableSnapshot {
            syspath: path.into(),
            pd_revision,
            identity,
            plug_modes,
        })
    }

    fn alt_modes(&self, owner: &Path) -> Result<Vec<AltMode>> {
        let owner_name = owner
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        let prefix = format!("{owner_name}.");
        let mut paths = Vec::new();
        for directory in [&self.root, owner] {
            let entries = match fs::read_dir(directory) {
                Ok(value) => value,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(Error::io(directory, e)),
            };
            for entry in entries {
                let entry = entry.map_err(|e| Error::io(directory, e))?;
                let name = entry.file_name();
                if name.to_string_lossy().starts_with(&prefix) {
                    paths.push(entry.path());
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths.into_iter().map(read_alt_mode).collect()
    }
}

fn is_port_name(name: &str) -> bool {
    name.strip_prefix("port")
        .is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
}
fn port_number(name: &str) -> Option<u32> {
    name.strip_prefix("port")?.parse().ok()
}

fn optional_text(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn required_text(path: &Path) -> Result<String> {
    optional_text(path)?.ok_or_else(|| Error::io(path, io::Error::from(io::ErrorKind::NotFound)))
}

fn parse_u32(path: &Path, value: &str) -> Result<u32> {
    let trimmed = value.trim();
    let parsed = trimmed
        .strip_prefix("0x")
        .map_or_else(|| trimmed.parse(), |hex| u32::from_str_radix(hex, 16));
    parsed.map_err(|_| Error::Parse {
        path: path.into(),
        value: value.into(),
        expected: "u32",
    })
}

fn parse_role(value: &str) -> DataRole {
    let active = value
        .split_whitespace()
        .find_map(|word| word.strip_prefix('[').and_then(|w| w.strip_suffix(']')))
        .unwrap_or(value);
    match active {
        "host" => DataRole::Host,
        "device" => DataRole::Device,
        _ => DataRole::Unknown,
    }
}

fn parse_usb_modes(value: &str) -> UsbModeSet {
    let mut result = UsbModeSet::default();
    for word in value.split_whitespace() {
        let active = word.starts_with('[') && word.ends_with(']');
        let token = word.trim_matches(['[', ']']);
        let mode = match token {
            "usb2" => Some(UsbMode::Usb2),
            "usb3" => Some(UsbMode::Usb3),
            "usb4" => Some(UsbMode::Usb4),
            _ => None,
        };
        if let Some(mode) = mode {
            result.available.push(mode);
            if active {
                result.active = Some(mode);
            }
        }
    }
    result
}

fn read_identity(owner: &Path) -> Result<Option<PdIdentity>> {
    let identity = owner.join("identity");
    if !identity.exists() {
        return Ok(None);
    }
    let id_path = identity.join("id_header");
    let Some(id_raw) = optional_text(&id_path)? else {
        return Ok(None);
    };
    let cert_path = identity.join("cert_stat");
    let product_path = identity.join("product");
    let id_header = parse_u32(&id_path, &id_raw)?;
    let cert_stat = optional_text(&cert_path)?
        .map(|v| parse_u32(&cert_path, &v))
        .transpose()?
        .unwrap_or(0);
    let product = optional_text(&product_path)?
        .map(|v| parse_u32(&product_path, &v))
        .transpose()?
        .unwrap_or(0);
    let mut product_type_vdos = [None; 3];
    for (index, slot) in product_type_vdos.iter_mut().enumerate() {
        let path = identity.join(format!("product_type_vdo{}", index + 1));
        *slot = optional_text(&path)?
            .map(|v| parse_u32(&path, &v))
            .transpose()?;
    }
    Ok(Some(PdIdentity {
        id_header,
        cert_stat,
        product,
        product_type_vdos,
    }))
}

fn read_alt_mode(path: PathBuf) -> Result<AltMode> {
    let svid_path = path.join("svid");
    let mode_path = path.join("mode");
    let vdo_path = path.join("vdo");
    let svid_raw = required_text(&svid_path)?;
    let svid =
        u16::from_str_radix(svid_raw.trim_start_matches("0x"), 16).map_err(|_| Error::Parse {
            path: svid_path,
            value: svid_raw,
            expected: "hex SVID",
        })?;
    let mode_raw = required_text(&mode_path)?;
    let mode = mode_raw.parse().map_err(|_| Error::Parse {
        path: mode_path,
        value: mode_raw,
        expected: "mode number",
    })?;
    let vdo = optional_text(&vdo_path)?
        .map(|v| parse_u32(&vdo_path, &v))
        .transpose()?
        .unwrap_or(0);
    let active =
        optional_text(&path.join("active"))?.is_some_and(|v| matches!(v.as_str(), "yes" | "1"));
    Ok(AltMode {
        syspath: path,
        svid,
        mode,
        vdo,
        active,
    })
}

fn active_mode(partner: Option<&PartnerSnapshot>) -> ActiveMode {
    let Some(partner) = partner else {
        return ActiveMode::None;
    };
    if partner.usb_modes.active == Some(UsbMode::Usb4) {
        return ActiveMode::Usb4;
    }
    if let Some(mode) = partner.alt_modes.iter().find(|m| m.active) {
        return match mode.svid {
            THUNDERBOLT_SVID => ActiveMode::Thunderbolt {
                syspath: mode.syspath.clone(),
            },
            DISPLAYPORT_SVID => ActiveMode::DisplayPort {
                syspath: mode.syspath.clone(),
            },
            svid => ActiveMode::Other {
                svid,
                syspath: mode.syspath.clone(),
            },
        };
    }
    ActiveMode::None
}

fn find_usb4_link(
    port: &Path,
    thunderbolt_root: Option<&Path>,
) -> Result<(Option<Usb4Link>, bool)> {
    let entries = fs::read_dir(port).map_err(|e| Error::io(port, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(port, e))?;
        if entry.file_name().to_string_lossy().starts_with("usb4_port") {
            return usb4_link(entry.path()).map(|link| (Some(link), true));
        }
    }

    let Some(thunderbolt_root) = thunderbolt_root.filter(|root| root.exists()) else {
        return Ok((None, false));
    };
    let Some(port_location) = physical_location(port)? else {
        return Ok((None, false));
    };
    for router in fs::read_dir(thunderbolt_root).map_err(|e| Error::io(thunderbolt_root, e))? {
        let router = router.map_err(|e| Error::io(thunderbolt_root, e))?.path();
        let Ok(children) = fs::read_dir(&router) else {
            continue;
        };
        for child in children.filter_map(|entry| entry.ok()) {
            if !child.file_name().to_string_lossy().starts_with("usb4_port") {
                continue;
            }
            let path = child.path();
            if physical_location(&path)? == Some(port_location.clone()) {
                return usb4_link(path).map(|link| (Some(link), true));
            }
        }
    }
    Ok((None, false))
}

fn usb4_link(syspath: PathBuf) -> Result<Usb4Link> {
    let target = fs::canonicalize(&syspath).map_err(|e| Error::io(&syspath, e))?;
    let domain_syspath = target
        .ancestors()
        .find(|p| {
            p.file_name().and_then(|v| v.to_str()).is_some_and(|n| {
                n.strip_prefix("domain").is_some_and(|tail| {
                    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
                })
            })
        })
        .map(Path::to_path_buf);
    Ok(Usb4Link {
        syspath,
        domain_syspath,
    })
}

fn physical_location(device: &Path) -> Result<Option<Vec<(&'static str, String)>>> {
    let directory = device.join("physical_location");
    if !directory.exists() {
        return Ok(None);
    }
    let mut location = Vec::new();
    for field in [
        "panel",
        "horizontal_position",
        "vertical_position",
        "dock",
        "lid",
    ] {
        if let Some(value) = optional_text(&directory.join(field))? {
            location.push((field, value));
        }
    }
    Ok((location.len() >= 3).then_some(location))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    Exit { path: PathBuf },
    EnterUsb4 { path: PathBuf },
    EnterAltMode { path: PathBuf, svid: u16, mode: u8 },
}

pub trait TypecControl {
    fn exit(&mut self, port: &PortSnapshot) -> Result<Operation>;
    fn enter(&mut self, port: &PortSnapshot, target: &Candidate) -> Result<Operation>;
}

#[derive(Debug)]
pub struct SysfsControl {
    dry_run: bool,
}
impl SysfsControl {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }
    fn write(&self, path: &Path, value: &str) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        file.write_all(value.as_bytes())
            .map_err(|e| Error::io(path, e))
    }
}

impl TypecControl for SysfsControl {
    fn exit(&mut self, port: &PortSnapshot) -> Result<Operation> {
        let path = match &port.active_mode {
            ActiveMode::Usb4 => port.partner.as_ref().map(|p| p.syspath.join("usb_mode")),
            ActiveMode::Thunderbolt { syspath }
            | ActiveMode::DisplayPort { syspath }
            | ActiveMode::Other { syspath, .. } => Some(syspath.join("active")),
            ActiveMode::None => None,
        }
        .ok_or_else(|| Error::Unsupported(format!("{} has no active mode to exit", port.name)))?;
        let value = if matches!(port.active_mode, ActiveMode::Usb4) {
            "usb2\n"
        } else {
            "0\n"
        };
        self.write(&path, value)?;
        Ok(Operation::Exit { path })
    }

    fn enter(&mut self, port: &PortSnapshot, target: &Candidate) -> Result<Operation> {
        match target {
            Candidate::Usb4 => {
                let path = port
                    .partner
                    .as_ref()
                    .ok_or_else(|| Error::Unsupported("USB4 without partner".into()))?
                    .syspath
                    .join("usb_mode");
                self.write(&path, "usb4\n")?;
                Ok(Operation::EnterUsb4 { path })
            }
            Candidate::Thunderbolt { svid, mode } | Candidate::DisplayPort { svid, mode } => {
                let alt = port
                    .partner
                    .as_ref()
                    .and_then(|p| {
                        p.alt_modes
                            .iter()
                            .find(|m| m.svid == *svid && m.mode == *mode)
                    })
                    .ok_or_else(|| {
                        Error::Unsupported(format!("mode {svid:04x}:{mode} disappeared"))
                    })?;
                let path = alt.syspath.join("active");
                self.write(&path, "1\n")?;
                Ok(Operation::EnterAltMode {
                    path,
                    svid: *svid,
                    mode: *mode,
                })
            }
            Candidate::OrdinaryUsb => Err(Error::Unsupported(
                "ordinary USB needs no entry request".into(),
            )),
        }
    }
}
