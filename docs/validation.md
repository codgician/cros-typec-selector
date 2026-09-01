# Validation matrix

Validation is split between deterministic fixture/state-machine coverage and hardware observation. A fixture pass proves policy and operation ordering; it does not claim that an unavailable peripheral completed physical mode entry.

| Scenario | Deterministic coverage | Hardware status on 2026-09-02 |
| --- | --- | --- |
| USB4, passive cable | `passive-usb4`, sanitized Studio Display USB4 capture | Patched-kernel hotplug issued one USB4 request; the nested downstream router appeared, boltd authorized it at 40 Gb/s, and video worked |
| USB4, active cable | `active-usb4` | Untested |
| DP-only/fallback | `displayport-only` | Initial hotplug DP entry produced stable video; fallback after failed USB4 did not restore video until a physical reconnect |
| TBT-only | `tbt3-only` | Untested |
| USB-only | `usb3-only` | Current non-PD USB attachment inspected; no modal request |
| Power-only | `power-only` | Untested |
| Missing e-marker | `missing-emarker` | Untested; fixture falls back without USB4 |
| Incomplete discovery | `incomplete-discovery` | Fixture waits; malformed sysfs returns contextual error without a write |
| Detach during transition | attach-generation and udev cancellation tests | Physical hotplug advanced exactly one detach and one attach generation and issued no stale-generation write |
| Daemon restart | active-USB4 idempotence test and repeated read-only reconciliation | Live daemon restart retained active USB4 with `Keep` and issued no operation |
| boltd stopped | backend authorization-boundary test | With boltd condition-blocked, the selector reached active USB4 while both raw router `authorized` values remained 0; restoring boltd changed both to 1 and video worked |
| Cold boot, AP reboot, rapid reattach, suspend/resume | generation/reconciliation model only | Patched-kernel hotplug passed; cold boot with device, AP reboot, rapid reattach, and suspend/resume remain untested |

Hardware testing exposed and fixed four userspace defects: the nonblocking udev monitor exiting on `EAGAIN`, attachment generations advancing for child-device removals, repeated writes after a candidate had already failed, and missing USB4 association when a connector lacks a direct link. Association now falls back to matching kernel physical-location topology. The first patched-kernel run then exposed that downstream routers are nested below the host-router directory; completion detection now handles that topology and the subsequent hotplug passed on the first request.

## Reproducible commands

```text
nix develop path:. -c cargo test
nix develop path:. -c cargo run -- inspect
nix develop path:. -c cargo run -- decide
nix develop path:. -c cargo run -- reconcile
systemctl is-active bolt.service
sudo systemctl stop bolt.service
systemctl is-active bolt.service
nix develop path:. -c cargo run -- reconcile
sudo systemctl start bolt.service
systemctl is-active bolt.service
nix flake check path:.
sudo nix run path:. -- daemon --live
```

The running patched kernel suppresses early DP entry: discovery completed with no active mode, then the selector issued one USB4 request. No EC firmware update was performed by this project.
