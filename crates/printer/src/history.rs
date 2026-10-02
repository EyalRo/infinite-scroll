//! Durable print history and lifetime counters.
//!
//! Every physical print attempt -- scheduled or from a queued job -- is
//! recorded here, so a phone (or the web UI) can show recent results and
//! lifetime statistics. Counters begin at the first run of this version;
//! earlier prints are not backfilled. Written atomically like `state.rs`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 200;
/// Dots per millimetre at the Arkscan 2054A's 203 DPI.
const DOTS_PER_MM: f64 = 203.0 / 25.4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Fired by the autoprint timer.
    Scheduled,
    /// Came from an accepted print job (single item or print-all).
    Job,
    /// A direct "print now" from the web UI (`POST /catalog/<id>/print`).
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub at: f64,
    pub origin: Origin,
    pub item_id: String,
    pub original_filename: String,
    pub job_id: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    /// Estimated paper consumed, from the job's `^LL` height.
    pub paper_mm: Option<f64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Counters {
    pub since: f64,
    pub prints_ok: u64,
    pub prints_failed: u64,
    pub scheduled_ok: u64,
    pub job_ok: u64,
    #[serde(default)]
    pub manual_ok: u64,
    pub paper_mm: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    pub counters: Counters,
    pub recent: Vec<Record>,
}

impl History {
    pub fn load(path: &Path) -> History {
        fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn record(&mut self, record: Record) {
        if self.counters.since == 0.0 {
            self.counters.since = record.at;
        }
        if record.success {
            self.counters.prints_ok += 1;
            self.counters.paper_mm += record.paper_mm.unwrap_or(0.0);
            match record.origin {
                Origin::Scheduled => self.counters.scheduled_ok += 1,
                Origin::Job => self.counters.job_ok += 1,
                Origin::Manual => self.counters.manual_ok += 1,
            }
        } else {
            self.counters.prints_failed += 1;
        }
        self.recent.push(record);
        if self.recent.len() > MAX_RECENT {
            let excess = self.recent.len() - MAX_RECENT;
            self.recent.drain(..excess);
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, serde_json::to_vec_pretty(self).unwrap())?;
        fs::rename(&temp_path, path)
    }
}

/// Estimated paper length in mm from a job's `^LL<height>` command.
pub fn paper_mm_from_zpl(zpl: &str) -> Option<f64> {
    let head = &zpl[..zpl.len().min(256)];
    let start = head.find("^LL")? + 3;
    let digits: String = head[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
    let dots: f64 = digits.parse().ok()?;
    Some((dots / DOTS_PER_MM * 100.0).round() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(success: bool, origin: Origin, paper: Option<f64>) -> Record {
        Record { at: 10.0, origin, item_id: "a".into(), original_filename: "a.png".into(), job_id: None, success, error: None, paper_mm: paper }
    }

    #[test]
    fn counters_track_outcomes_origin_and_paper() {
        let mut h = History::default();
        h.record(rec(true, Origin::Scheduled, Some(81.0)));
        h.record(rec(true, Origin::Job, Some(19.0)));
        h.record(rec(false, Origin::Job, None));
        assert_eq!((h.counters.prints_ok, h.counters.prints_failed), (2, 1));
        assert_eq!((h.counters.scheduled_ok, h.counters.job_ok), (1, 1));
        assert_eq!(h.counters.paper_mm, 100.0);
        assert_eq!(h.counters.since, 10.0);
    }

    #[test]
    fn recent_is_capped_but_counters_are_lifetime() {
        let mut h = History::default();
        for _ in 0..(MAX_RECENT + 10) {
            h.record(rec(true, Origin::Job, Some(1.0)));
        }
        assert_eq!(h.recent.len(), MAX_RECENT);
        assert_eq!(h.counters.prints_ok, (MAX_RECENT + 10) as u64);
    }

    #[test]
    fn paper_length_from_ll_matches_the_recorded_smoke_test() {
        // 650-wide Akita print: 650 x 650 dots is ~81.3 mm at 203 DPI.
        let mm = paper_mm_from_zpl("^XA\n^PW650\n^LL650\n^LH0,0\n").unwrap();
        assert!((mm - 81.33).abs() < 0.01, "{mm}");
        assert!(paper_mm_from_zpl("^XA\n^XZ\n").is_none());
    }

    #[test]
    fn save_load_round_trip() {
        let path = std::env::temp_dir().join(format!("history-test-{}.json", std::process::id()));
        let mut h = History::default();
        h.record(rec(true, Origin::Job, Some(5.0)));
        h.save(&path).unwrap();
        assert_eq!(History::load(&path).counters.prints_ok, 1);
        let _ = fs::remove_file(&path);
    }
}
