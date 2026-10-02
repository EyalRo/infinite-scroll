//! Capability-based printer settings.
//!
//! A setting appears here only after it has been confirmed against the
//! physical Arkscan 2054A over the validated raw-USB path
//! (`/dev/usb/lp0`, ZPL) -- see `docs/printer-settings.md` for the probe
//! procedure and recorded findings. There is deliberately no generic
//! "send this command to the printer" entry point: each confirmed setting
//! gets its own typed read/write implementation.
//!
//! As of this version no setting has been verified, so the capability list
//! is empty and `set` rejects every key. Print density/darkness is the
//! first candidate once the probe has been run on hardware.

use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct Capability {
    pub key: &'static str,
    pub label: &'static str,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub readable: bool,
    pub writable: bool,
}

/// Confirmed settings. Extend only with hardware-verified entries.
pub fn capabilities() -> Vec<Capability> {
    Vec::new()
}

#[derive(Debug, PartialEq, Eq)]
pub enum SettingError {
    Unsupported,
    OutOfRange { min: i64, max: i64 },
}

pub fn set(key: &str, value: i64) -> Result<(), SettingError> {
    let capability = capabilities().into_iter().find(|c| c.key == key && c.writable).ok_or(SettingError::Unsupported)?;
    if value < capability.min || value > capability.max || (value - capability.min) % capability.step != 0 {
        return Err(SettingError::OutOfRange { min: capability.min, max: capability.max });
    }
    // No verified write path exists yet; unreachable until capabilities()
    // gains an entry, at which point this must perform that entry's write.
    Err(SettingError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_advertised_until_verified_on_hardware() {
        assert!(capabilities().is_empty());
    }

    #[test]
    fn unknown_settings_are_rejected() {
        assert_eq!(set("density", 10), Err(SettingError::Unsupported));
        assert_eq!(set("anything-else", 1), Err(SettingError::Unsupported));
    }
}
