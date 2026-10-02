//! Capability-based printer settings.
//!
//! A setting appears here only after it has been confirmed against the
//! physical Arkscan 2054A over the validated raw-USB path
//! (`/dev/usb/lp0`, ZPL) -- see `docs/printer-settings.md` for the probe
//! procedure and recorded findings. There is deliberately no generic
//! "send this command to the printer" entry point: each confirmed setting
//! (and action) gets its own typed implementation.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::printer_device;

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

/// One-shot things the printer can be told to do (as opposed to a value).
#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub key: &'static str,
    pub label: &'static str,
}

pub const DARKNESS: &str = "darkness";
pub const PRINT_CONFIG: &str = "print_config";

/// Confirmed settings. Extend only with hardware-verified entries.
pub fn capabilities() -> Vec<Capability> {
    vec![Capability { key: DARKNESS, label: "Print darkness", min: 0, max: 30, step: 1, readable: true, writable: true }]
}

/// Confirmed actions. Extend only with hardware-verified entries.
pub fn actions() -> Vec<Action> {
    vec![Action { key: PRINT_CONFIG, label: "Print settings label" }]
}

#[derive(Debug, PartialEq, Eq)]
pub enum SettingError {
    Unsupported,
    OutOfRange { min: i64, max: i64 },
    /// The printer could not be reached, or did not take the value.
    Device(String),
}

/// How long to wait for the printer's configuration report.
const REPORT_WAIT: Duration = Duration::from_secs(4);

/// Extracts `DARKNESS` from a `^HH` configuration report, e.g.
/// `  10.0                DARKNESS          `.
pub fn parse_darkness(report: &str) -> Option<i64> {
    report
        .lines()
        .find(|line| line.contains("DARKNESS"))
        .and_then(|line| line.trim_matches(|c: char| c.is_control() || c == ' ').split_whitespace().next().map(str::to_string))
        .and_then(|token| token.parse::<f64>().ok())
        .map(|value| value.round() as i64)
}

/// Reads the current darkness from the printer itself (`^HH` replies on the
/// return channel without printing anything).
pub fn read_darkness(device_path: &Path) -> Result<i64, SettingError> {
    let report = printer_device::query(device_path, "^XA^HH^XZ", REPORT_WAIT).map_err(SettingError::Device)?;
    parse_darkness(&report).ok_or_else(|| SettingError::Device("the printer's configuration report had no DARKNESS line".to_string()))
}

/// Writes `value` with `~SD`, remembers it so every later job re-applies it,
/// then reads it back so a printer that ignored the command is reported.
pub fn set(device_path: &Path, key: &str, value: i64) -> Result<i64, SettingError> {
    let capability = capabilities().into_iter().find(|c| c.key == key && c.writable).ok_or(SettingError::Unsupported)?;
    if value < capability.min || value > capability.max || (value - capability.min) % capability.step != 0 {
        return Err(SettingError::OutOfRange { min: capability.min, max: capability.max });
    }
    printer_device::set_darkness(Some(value as u8));
    let sent = printer_device::print_zpl(device_path, &format!("~SD{value:02}"), Duration::from_secs(15));
    if !sent.success {
        return Err(SettingError::Device(sent.message));
    }
    let now = read_darkness(device_path)?;
    if now != value {
        return Err(SettingError::Device(format!("the printer reports darkness {now} after being set to {value}")));
    }
    Ok(now)
}

/// Makes the printer print its own settings label (`~WC`).
pub fn print_config(device_path: &Path) -> Result<(), SettingError> {
    let sent = printer_device::print_zpl(device_path, "~WC", Duration::from_secs(15));
    if sent.success {
        Ok(())
    } else {
        Err(SettingError::Device(sent.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "\u{2}  10.0                DARKNESS          \r\n  6 IPS               PRINT SPEED       \r\n";

    #[test]
    fn darkness_is_the_only_confirmed_setting() {
        let keys: Vec<_> = capabilities().iter().map(|c| c.key).collect();
        assert_eq!(keys, vec!["darkness"]);
        assert_eq!(actions().iter().map(|a| a.key).collect::<Vec<_>>(), vec!["print_config"]);
    }

    #[test]
    fn parses_darkness_from_a_configuration_report() {
        assert_eq!(parse_darkness(REPORT), Some(10));
        assert_eq!(parse_darkness("  25.0   DARKNESS \r\n"), Some(25));
        assert_eq!(parse_darkness("  6 IPS   PRINT SPEED\r\n"), None);
        assert_eq!(parse_darkness(""), None);
    }

    #[test]
    fn unknown_settings_are_rejected() {
        assert_eq!(set(Path::new("/dev/null"), "density", 10), Err(SettingError::Unsupported));
        assert_eq!(set(Path::new("/dev/null"), "anything-else", 1), Err(SettingError::Unsupported));
    }

    #[test]
    fn out_of_range_values_are_rejected_before_touching_the_printer() {
        let missing = Path::new("/does/not/exist");
        assert_eq!(set(missing, DARKNESS, 31), Err(SettingError::OutOfRange { min: 0, max: 30 }));
        assert_eq!(set(missing, DARKNESS, -1), Err(SettingError::OutOfRange { min: 0, max: 30 }));
    }
}
