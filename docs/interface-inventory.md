# Type-C interface inventory

This selector uses only the Linux Type-C connector class rooted at `/sys/class/typec`.
Names are discovered by enumeration; no connector or USB4 domain number is assumed.

## Reads

- `portN/data_role`: normalized current data role.
- `portN/usb4_port*`: a direct link identifies host USB4 capability and its domain. If that link is absent, host-router `usb4_port*` objects are matched by the kernel-exported physical-location tuple; numeric connector/domain suffixes are never correlated.
- `portN-partner/supports_usb_power_delivery` and `usb_power_delivery_revision`.
- `portN-partner/identity/{id_header,cert_stat,product,product_type_vdo1..3}`.
- `portN-partner/usb_mode`: supported choices and bracketed active USB mode.
- `portN-partner.M/{svid,mode,vdo,active}`: partner alternate modes.
- `portN-cable/identity/*` and `usb_power_delivery_revision`.
- `portN-plug0.M/{svid,mode,vdo,active}`: SOP-prime alternate modes.
- Associated USB4 domain children: physical downstream-router enumeration only; authorization attributes are never read as policy inputs.

Every optional discovery object may be absent. A disappearing required object invalidates that reconciliation. Numeric values accept decimal or `0x` hexadecimal where the kernel ABI uses both.

## Writes

The backend exposes only these operations:

- USB4 enter: write `usb4` to `portN-partner/usb_mode`.
- USB4 exit: write `usb2` to the same attribute (the connector-class exit request).
- DP/TBT enter: write `1` to the selected `portN-partner.M/active`.
- DP/TBT exit: write `0` to that same `active` attribute.

There is no Thunderbolt-bus backend and no operation that can write `authorized`.
Writes are root-owned on the inspected host (`-rw-r--r--`) and require the explicit `--live` CLI flag.

## Inspected host state (2026-09-01)

- Two dynamically enumerated connectors; AP-driven entry reports `yes` at `/sys/class/chromeos/cros_ec/ap_mode_entry`.
- The attached object at capture time was non-PD (`supports_usb_power_delivery=no`, revision `0.0`), active `usb2`, with zero identity VDOs and no cable/SOP-prime object. This is intentionally not labelled with a vendor/product identity.
- The host connector exposed a `usb4_port*` link whose canonical target lies below a USB4 domain. The association is therefore derived from the link, not matching numeric suffixes.
- Host DP and TBT alternate-mode interfaces were present. Partner alternate modes were absent for the attached non-PD object.

## Controlled live experiment

Do this only with console access and after saving the read-only capture:

```text
cros-typec-selector inspect PORT
cros-typec-selector decide PORT
cros-typec-selector reconcile PORT                 # dry-run
sudo cros-typec-selector reconcile PORT --live
cros-typec-selector inspect PORT
boltctl list
```

On a kernel carrying the coordination patch, expected DP-to-USB4 operation order is `active=0`, observe no modal state, `usb_mode=usb4`, then observe USB4 plus a downstream router in the associated domain. On an uncoordinated kernel where DP is already active, live mode now keeps DP and reports `coordination-required` rather than risking a disruptive replacement. Roll back an interrupted development experiment with `echo usb2 | sudo tee /sys/class/typec/PORT-partner/usb_mode`; if DP is required, write `1` to the previously recorded DP partner-altmode `active` path. Never write any Thunderbolt `authorized` attribute. Capture kernel/EC logs around the experiment and redact serial numbers before publishing.
