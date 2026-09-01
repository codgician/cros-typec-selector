use cros_typec_selector::daemon::{Daemon, Mode};
use cros_typec_selector::error::{Error, Result};
use cros_typec_selector::policy::{self, Context};
use cros_typec_selector::sysfs::Sysfs;
use std::env;
use std::path::PathBuf;

fn usage() -> &'static str {
    "usage:\n  cros-typec-selector inspect [PORT] [--sysfs PATH]\n  cros-typec-selector decide [PORT] [--sysfs PATH] [--discovery-expired]\n  cros-typec-selector reconcile [PORT] [--sysfs PATH] [--live]\n  cros-typec-selector daemon [--sysfs PATH] --live\n\nWrites are disabled unless --live is explicit."
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        println!("{}", usage());
        return Ok(());
    }
    let command = args.remove(0);
    let sysfs_root = take_value(&mut args, "--sysfs")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/sys/class/typec"));
    let live = take_flag(&mut args, "--live");
    let expired = take_flag(&mut args, "--discovery-expired");
    let port = args.first().filter(|a| !a.starts_with('-')).cloned();
    if args.iter().any(|a| a.starts_with('-')) || args.len() > usize::from(port.is_some()) {
        return Err(Error::Unsupported(usage().into()));
    }
    let sysfs = Sysfs::new(sysfs_root);
    match command.as_str() {
        "inspect" => {
            for snapshot in select(&sysfs, port.as_deref())? {
                print!("{snapshot}");
            }
        }
        "decide" => {
            for snapshot in select(&sysfs, port.as_deref())? {
                println!(
                    "port={} candidates={:?} decision={:?}",
                    snapshot.name,
                    policy::candidates(
                        &snapshot,
                        Context {
                            discovery_expired: expired
                        }
                    ),
                    policy::decide(
                        &snapshot,
                        Context {
                            discovery_expired: expired
                        }
                    )
                );
            }
        }
        "reconcile" => {
            let mut daemon = Daemon::new(sysfs, if live { Mode::Live } else { Mode::DryRun });
            let lines = if let Some(port) = port {
                daemon.reconcile_port(&port)?
            } else {
                daemon.reconcile_all()?
            };
            for line in lines {
                println!("{line}");
            }
        }
        "daemon" => {
            if !live {
                return Err(Error::Unsupported("daemon requires explicit --live; use inspect/decide/reconcile for read-only operation".into()));
            }
            Daemon::new(sysfs, Mode::Live).run()?;
        }
        _ => return Err(Error::Unsupported(usage().into())),
    }
    Ok(())
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(index) = args.iter().position(|a| a == flag) {
        args.remove(index);
        true
    } else {
        false
    }
}
fn take_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let index = args.iter().position(|a| a == flag)?;
    args.remove(index);
    if index < args.len() {
        Some(args.remove(index))
    } else {
        None
    }
}
fn select(
    sysfs: &Sysfs,
    port: Option<&str>,
) -> Result<Vec<cros_typec_selector::topology::PortSnapshot>> {
    match port {
        Some(name) => Ok(vec![sysfs.port(name)?]),
        None => sysfs.ports(),
    }
}
