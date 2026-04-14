//! USB printer discovery via `Get-PnpDevice`.
//!
//! Returns every USB-attached printing device Windows knows about — both
//! working queues and yellow-bang orphans that PnP could not auto-install
//! a driver for. The caller cross-references queue state and can use the
//! result to drive the `add --usb` flow for legacy printers.
