use crate::error::{Error, Result};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventHint {
    pub port: Option<String>,
    pub action: udev::EventType,
    pub partner: bool,
}

pub struct Monitor {
    socket: udev::MonitorSocket,
}

impl Monitor {
    pub fn new() -> Result<Self> {
        let socket = udev::MonitorBuilder::new()
            .and_then(|builder| builder.match_subsystem("typec"))
            .and_then(|builder| builder.listen())
            .map_err(|error| Error::io("udev monitor", error))?;
        Ok(Self { socket })
    }

    pub fn wait(&self, timeout: Option<Duration>) -> Result<bool> {
        let timeout = timeout
            .map(PollTimeout::try_from)
            .transpose()
            .map_err(|error| Error::Unsupported(format!("invalid udev poll timeout: {error}")))?
            .unwrap_or(PollTimeout::NONE);
        // SAFETY: `self.socket` owns this descriptor and outlives the PollFd.
        let fd = unsafe { BorrowedFd::borrow_raw(self.socket.as_raw_fd()) };
        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        poll(&mut fds, timeout).map_err(|error| {
            Error::io("udev monitor", io::Error::from_raw_os_error(error as i32))
        })?;
        let events = fds[0].revents().unwrap_or(PollFlags::empty());
        if events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
            return Err(Error::Unsupported(format!(
                "udev monitor poll failure: {events:?}"
            )));
        }
        Ok(events.contains(PollFlags::POLLIN))
    }

    pub fn drain(&self) -> Vec<EventHint> {
        self.socket.iter().map(normalize).collect()
    }
}

fn normalize(event: udev::Event) -> EventHint {
    let port = port_from_syspath(event.syspath());
    EventHint {
        partner: is_partner_path(event.syspath(), port.as_deref()),
        port,
        action: event.event_type(),
    }
}

pub fn port_from_syspath(path: &Path) -> Option<String> {
    path.components()
        .filter_map(|part| part.as_os_str().to_str())
        .find_map(|name| {
            let base = name.split(['-', '.']).next()?;
            base.strip_prefix("port")
                .filter(|tail| !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit()))
                .map(|_| base.to_owned())
        })
}

fn is_partner_path(path: &Path, port: Option<&str>) -> bool {
    port.is_some_and(|port| {
        path.file_name().and_then(|name| name.to_str()) == Some(format!("{port}-partner").as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_nested_typec_paths() {
        assert_eq!(
            port_from_syspath(Path::new(
                "/devices/x/typec/port12/port12-partner/port12-partner.0"
            )),
            Some("port12".into())
        );
    }

    #[test]
    fn only_partner_object_changes_attachment() {
        assert!(is_partner_path(
            Path::new("/devices/typec/port4/port4-partner"),
            Some("port4")
        ));
        assert!(!is_partner_path(
            Path::new("/devices/typec/port4/port4-partner/port4-partner.1"),
            Some("port4")
        ));
    }
}
