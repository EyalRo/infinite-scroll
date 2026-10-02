//! Wi-Fi helpers: parsing `nmcli -t` output and validating what a phone sends.
//!
//! Everything here is pure so it can be tested without NetworkManager. The
//! operations themselves (and the rollback when a connection fails) live in
//! `handler.rs`.

use serde_json::{json, Value};

/// Profiles this service creates are named with this prefix, so it only ever
/// deletes or replaces profiles it made itself.
pub const PROFILE_PREFIX: &str = "infinite-scroll-";
pub const MAX_NETWORKS: usize = 30;

/// Splits one line of `nmcli -t` output. Fields are separated by `:`, and a
/// literal `:` or `\` inside a field is escaped with a backslash.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().unwrap().push(next);
                }
            }
            ':' => fields.push(String::new()),
            other => fields.last_mut().unwrap().push(other),
        }
    }
    fields
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// 0-100, the strongest access point seen for this name.
    pub signal: u8,
    /// `open`, `wpa` (PSK: WPA/WPA2/WPA3 personal) or `enterprise` (unsupported).
    pub security: &'static str,
    pub in_use: bool,
}

pub fn classify_security(field: &str) -> &'static str {
    let field = field.trim();
    if field.is_empty() || field == "--" {
        "open"
    } else if field.contains("802.1X") {
        "enterprise"
    } else {
        "wpa"
    }
}

/// Parses `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list`. A
/// network usually appears once per access point; they are merged by name,
/// keeping the strongest signal and the in-use flag. Hidden (empty-name)
/// entries are dropped. Sorted: in use first, then strongest.
pub fn parse_networks(output: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let fields = split_terse(line);
        if fields.len() < 4 || fields[1].trim().is_empty() {
            continue;
        }
        let signal = fields[2].trim().parse::<u32>().unwrap_or(0).min(100) as u8;
        let in_use = fields[0].trim() == "*";
        let security = classify_security(&fields[3]);
        match networks.iter_mut().find(|n| n.ssid == fields[1]) {
            Some(existing) => {
                existing.signal = existing.signal.max(signal);
                existing.in_use |= in_use;
            }
            None => networks.push(Network { ssid: fields[1].clone(), signal, security, in_use }),
        }
    }
    networks.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)).then(a.ssid.cmp(&b.ssid)));
    networks.truncate(MAX_NETWORKS);
    networks
}

pub fn networks_json(networks: &[Network]) -> Value {
    json!(networks
        .iter()
        .map(|n| json!({"ssid": n.ssid, "signal": n.signal, "security": n.security, "in_use": n.in_use}))
        .collect::<Vec<_>>())
}

/// The connected network, if any, from the same `device wifi list` output.
pub fn current(networks: &[Network]) -> Option<&Network> {
    networks.iter().find(|n| n.in_use)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Invalid {
    Ssid,
    Password,
}

/// 1-32 bytes, no control characters.
pub fn validate_ssid(ssid: &str) -> Result<(), Invalid> {
    if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(|c| c.is_control()) {
        return Err(Invalid::Ssid);
    }
    Ok(())
}

/// WPA-personal: an 8-63 character passphrase, or exactly 64 hex digits.
/// Empty means an open network and is accepted here.
pub fn validate_password(password: &str) -> Result<(), Invalid> {
    if password.is_empty() {
        return Ok(());
    }
    let hex64 = password.len() == 64 && password.chars().all(|c| c.is_ascii_hexdigit());
    let passphrase = (8..=63).contains(&password.chars().count()) && password.chars().all(|c| !c.is_control());
    if hex64 || passphrase {
        Ok(())
    } else {
        Err(Invalid::Password)
    }
}

pub fn profile_name(ssid: &str) -> String {
    format!("{PROFILE_PREFIX}{ssid}")
}

/// Turns nmcli's complaint into something a person can act on. The raw text
/// is never returned (it can name the network or echo settings).
pub fn explain_failure(stderr: &str) -> &'static str {
    let text = stderr.to_ascii_lowercase();
    if text.contains("secrets were required") || text.contains("no secrets") || text.contains("password") || text.contains("802-1x") || text.contains("4-way") {
        "The network rejected the password."
    } else if text.contains("no network with ssid") || text.contains("not found") || text.contains("no suitable") {
        "The network is not in range."
    } else if text.contains("timeout") || text.contains("timed out") {
        "The network did not answer in time."
    } else {
        "Could not connect to the network."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "*:Fred is SPEED:61:WPA2 WPA3\n :Fred is SPEED:80:WPA2 WPA3\n :Cafe\\: Guest:40:--\n :Corp:70:WPA2 802.1X\n :  :90:WPA2\n::55:WPA2\n";

    #[test]
    fn terse_lines_split_on_unescaped_colons() {
        assert_eq!(split_terse("a:b\\:c:d"), vec!["a", "b:c", "d"]);
        assert_eq!(split_terse("x\\\\y:z"), vec!["x\\y", "z"]);
        assert_eq!(split_terse(""), vec![""]);
    }

    #[test]
    fn access_points_merge_by_name_and_keep_the_strongest_signal() {
        let networks = parse_networks(LIST);
        let fred = networks.iter().find(|n| n.ssid == "Fred is SPEED").unwrap();
        assert_eq!(fred.signal, 80);
        assert!(fred.in_use);
        assert_eq!(networks.iter().filter(|n| n.ssid == "Fred is SPEED").count(), 1);
    }

    #[test]
    fn the_connected_network_comes_first_then_strongest() {
        let names: Vec<_> = parse_networks(LIST).into_iter().map(|n| n.ssid).collect();
        assert_eq!(names, vec!["Fred is SPEED", "Corp", "Cafe: Guest"]);
    }

    #[test]
    fn security_is_classified_and_colons_in_names_survive() {
        let networks = parse_networks(LIST);
        assert_eq!(networks.iter().find(|n| n.ssid == "Cafe: Guest").unwrap().security, "open");
        assert_eq!(networks.iter().find(|n| n.ssid == "Corp").unwrap().security, "enterprise");
        assert_eq!(networks.iter().find(|n| n.ssid == "Fred is SPEED").unwrap().security, "wpa");
    }

    #[test]
    fn hidden_networks_are_dropped_and_the_current_one_is_found() {
        let networks = parse_networks(LIST);
        assert!(networks.iter().all(|n| !n.ssid.is_empty()));
        assert_eq!(current(&networks).unwrap().ssid, "Fred is SPEED");
        assert!(current(&parse_networks(" :Other:50:WPA2\n")).is_none());
    }

    #[test]
    fn the_list_is_bounded() {
        let many: String = (0..100).map(|i| format!(" :net{i}:{}:WPA2\n", i % 100)).collect();
        assert_eq!(parse_networks(&many).len(), MAX_NETWORKS);
    }

    #[test]
    fn ssid_validation() {
        assert!(validate_ssid("Fred is SPEED").is_ok());
        assert!(validate_ssid("-looks-like-an-option").is_ok());
        assert_eq!(validate_ssid(""), Err(Invalid::Ssid));
        assert_eq!(validate_ssid(&"x".repeat(33)), Err(Invalid::Ssid));
        assert_eq!(validate_ssid("bad\nname"), Err(Invalid::Ssid));
    }

    #[test]
    fn password_validation() {
        assert!(validate_password("").is_ok());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password(&"a".repeat(63)).is_ok());
        assert!(validate_password(&"ab".repeat(32)).is_ok()); // 64 hex digits
        assert_eq!(validate_password("short"), Err(Invalid::Password));
        assert_eq!(validate_password(&"g".repeat(64)), Err(Invalid::Password));
        assert_eq!(validate_password("has\nnewline12"), Err(Invalid::Password));
    }

    #[test]
    fn failures_are_explained_without_echoing_nmcli() {
        assert_eq!(explain_failure("Error: Connection activation failed: Secrets were required, but not provided."), "The network rejected the password.");
        assert_eq!(explain_failure("Error: No network with SSID 'x' found."), "The network is not in range.");
        assert_eq!(explain_failure("Error: Timeout expired (30 seconds)"), "The network did not answer in time.");
        assert_eq!(explain_failure("something odd"), "Could not connect to the network.");
    }

    #[test]
    fn profile_names_are_prefixed() {
        assert_eq!(profile_name("Home"), "infinite-scroll-Home");
    }
}
