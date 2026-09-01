# Policy and ownership boundary

`cros-typec-selector` owns only connector-class mode selection when Chrome EC advertises AP-driven mode entry. Pure policy consumes normalized USB-PD identity, partner/cable alternate modes, and host capability. Its priority is USB4, Thunderbolt compatibility, DisplayPort, then ordinary USB. Missing discovery remains a wait condition until a bounded deadline; after expiry, uncertain high-speed candidates are omitted rather than inferred.

The sysfs edge can request USB4 through `usb_mode` and DP/TBT through partner-altmode `active`. It cannot authorize a Thunderbolt router. `boltd` remains the only authorization and tunnel-policy owner, including when it is stopped or restarted.

Udev events trigger a fresh topology read. They are never treated as complete topology. Every attachment has a generation, and every delayed write checks that generation plus the udev event epoch before acting.

The program is read-only on any Linux Type-C system for `inspect`, `decide`, and default `reconcile`. Live writes require `--live` and connector-class controls supported by the driver.

## libtypec evaluation

The current libtypec public C API exposes discovered identity, alternate modes, cable properties, and udev callbacks, but its sysfs backend constructs connector paths from numeric indices and does not expose the `usb_mode` connector-class control needed here. Adopting it now would add an FFI boundary without replacing this selector's generation-safe snapshots or actuator. The reusable contribution point is a path-based, race-aware topology snapshot/control API; once libtypec provides that boundary, `sysfs.rs` can be replaced without changing pure policy.
