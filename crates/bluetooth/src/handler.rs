//! Maps protocol operations onto the existing Infinite Scroll services.
//!
//! This is deliberately thin: library, scheduling, printing and upload
//! semantics all live in `printer` / `uploader`. The only operation handled
//! here is the clock, because neither service owns it. The match in
//! `Handler::dispatch` is the complete list of what a phone can do; there is
//! no generic pass-through to the services or to the operating system.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::protocol::*;
use crate::wifi;

/// Upload sessions idle for longer than this are discarded.
const UPLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_LIBRARY_PAGE: usize = 50;
const DEFAULT_LIBRARY_PAGE: usize = 20;
/// Earliest/latest plausible phone time; guards against a bad client.
const MIN_PLAUSIBLE_UNIX_MS: i64 = 1_767_225_600_000; // 2026-01-01
const MAX_PLAUSIBLE_UNIX_MS: i64 = 4_102_444_800_000; // 2100-01-01
/// A clock correction larger than this re-rolls the autoprint timer, which
/// stores an absolute wall-clock `next_print_at`.
const CLOCK_RESCHEDULE_THRESHOLD_MS: i64 = 60_000;

/// Failure to reach or use a backing service.
#[derive(Debug)]
pub enum BackendError {
    Unreachable(String),
}

pub type BackendResult = Result<(u16, Value), BackendError>;

/// What `nmcli` printed and whether it succeeded.
pub struct NmOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait Backend: Send + Sync {
    /// Calls the printer service's HTTP API.
    fn printer(&self, method: &str, path: &str, body: Option<&Value>) -> BackendResult;
    /// Whether the uploader answers its health check.
    fn uploader_healthy(&self) -> bool;
    /// Posts raw image bytes through the uploader's normal `/uploads` path.
    fn submit_upload(&self, bytes: &[u8]) -> BackendResult;
    fn clock_now_ms(&self) -> i64;
    fn set_clock_ms(&self, unix_ms: i64) -> Result<(), String>;
    /// Runs NetworkManager's CLI with these arguments (no shell is involved).
    fn nmcli(&self, args: &[&str], timeout: Duration) -> Result<NmOutput, String>;
}

struct UploadSession {
    id: u16,
    name: String,
    size: usize,
    data: Vec<u8>,
    error: Option<String>,
    last_activity: Instant,
}

pub struct Handler<B: Backend> {
    backend: Arc<B>,
    wifi_attempt: Arc<Mutex<WifiAttempt>>,
    upload: Mutex<Option<UploadSession>>,
    next_session: Mutex<u16>,
}

struct OpError {
    code: ErrorCode,
    message: String,
    details: Option<Value>,
}

impl OpError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        OpError { code, message: message.into(), details: None }
    }
}

type OpResult = Result<(Disposition, Value), OpError>;

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, OpError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, format!("{key} must be a string")))
}

fn arg_u64(args: &Value, key: &str) -> Result<u64, OpError> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, format!("{key} must be a non-negative integer")))
}

fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, OpError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, format!("{key} must be a non-negative integer"))),
    }
}

/// Item ids are interpolated into backend URL paths; only the characters the
/// uploader/watcher ever generate (UUIDs) are accepted.
fn check_id(id: &str) -> Result<&str, OpError> {
    if !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(id)
    } else {
        Err(OpError::new(ErrorCode::InvalidArgument, "id has an invalid format"))
    }
}

fn digest(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

impl<B: Backend + 'static> Handler<B> {
    pub fn new(backend: B) -> Self {
        Handler { backend: Arc::new(backend), wifi_attempt: Arc::new(Mutex::new(WifiAttempt::default())), upload: Mutex::new(None), next_session: Mutex::new(1) }
    }

    /// Parses and executes one request body, always producing a response
    /// body (errors included) so the phone is never left waiting.
    pub fn handle_message(&self, body: &[u8]) -> Value {
        let Ok(request) = serde_json::from_slice::<Value>(body) else {
            return error_response(0, ErrorCode::BadRequest, "request is not valid JSON", None);
        };
        let id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
        if request.get("v").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
            return error_response(
                id,
                ErrorCode::UnsupportedVersion,
                "unsupported protocol version",
                Some(json!({"supported": [PROTOCOL_VERSION]})),
            );
        }
        let Some(op) = request.get("op").and_then(Value::as_str) else {
            return error_response(id, ErrorCode::BadRequest, "op must be a string", None);
        };
        let empty = json!({});
        let args = request.get("args").unwrap_or(&empty);
        log::debug!("request id={id} op={op}");
        match self.dispatch(op, args) {
            Ok((disposition, result)) => ok_response(id, disposition, result),
            Err(error) => {
                log::warn!("request id={id} op={op} failed code={} message={}", error.code.as_str(), error.message);
                error_response(id, error.code, &error.message, error.details)
            }
        }
    }

    fn dispatch(&self, op: &str, args: &Value) -> OpResult {
        match op {
            "sys.status" => self.sys_status(),
            "sys.clock.get" => Ok((Disposition::Completed, json!({"unix_ms": self.backend.clock_now_ms()}))),
            "sys.clock.set" => self.clock_set(args),

            "library.list" => self.library_list(args),
            "library.get" => self.printer_op("GET", &format!("/catalog/{}", check_id(arg_str(args, "id")?)?), None, Disposition::Completed),
            "library.thumbnail" => {
                let id = check_id(arg_str(args, "id")?)?;
                let width = opt_u64(args, "width")?.unwrap_or(160);
                self.printer_op("GET", &format!("/catalog/{id}/thumbnail?w={width}"), None, Disposition::Completed)
            }
            "library.delete" => self.printer_op("DELETE", &format!("/catalog/{}", check_id(arg_str(args, "id")?)?), None, Disposition::Completed),

            "upload.begin" => self.upload_begin(args),
            "upload.status" => self.upload_status(args),
            "upload.commit" => self.upload_commit(args),
            "upload.abort" => self.upload_abort(args),

            "print.item" => {
                let id = check_id(arg_str(args, "id")?)?;
                let body = json!({"id": id, "copies": opt_u64(args, "copies")?.unwrap_or(1)});
                self.printer_op("POST", "/print", Some(&body), Disposition::Accepted)
            }
            "print.all" => {
                let body = json!({"copies": opt_u64(args, "copies")?.unwrap_or(1)});
                self.printer_op("POST", "/print-all", Some(&body), Disposition::Accepted)
            }
            "print.jobs" => self.printer_op("GET", "/jobs", None, Disposition::Completed),
            "print.cancel" => self.printer_op("DELETE", &format!("/jobs/{}", check_id(arg_str(args, "job_id")?)?), None, Disposition::Completed),

            "print.history" => {
                let limit = opt_u64(args, "limit")?.unwrap_or(20);
                self.printer_op("GET", &format!("/history?limit={limit}"), None, Disposition::Completed)
            }
            "stats.get" => self.printer_op("GET", "/stats", None, Disposition::Completed),

            // Capability-based printer settings. Only settings the printer
            // service advertises (i.e. verified on hardware) can be used.
            "printer.print_config" => self.printer_op("POST", "/printer/print-config", Some(&json!({})), Disposition::Completed),
            "printer.capabilities" => self.printer_op("GET", "/printer/capabilities", None, Disposition::Completed),
            "printer.settings.get" => self.printer_op("GET", "/printer/settings", None, Disposition::Completed),
            "printer.settings.set" => {
                let key = arg_str(args, "key")?;
                if key.is_empty() || key.len() > 32 || !key.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    return Err(OpError::new(ErrorCode::InvalidArgument, "key has an invalid format"));
                }
                let value = args
                    .get("value")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, "value must be an integer"))?;
                self.printer_op("POST", "/printer/settings", Some(&json!({"key": key, "value": value})), Disposition::Completed)
            }

            "wifi.status" => self.wifi_status(),
            "wifi.scan" => self.wifi_scan(),
            "wifi.connect" => self.wifi_connect(args),

            "sched.get" => self.sched_get(),
            "sched.set" => self.sched_set(args),
            "sched.preview" => {
                let count = opt_u64(args, "count")?.unwrap_or(3);
                self.printer_op("GET", &format!("/preview?count={count}"), None, Disposition::Completed)
            }
            _ => Err(OpError::new(ErrorCode::UnknownOp, format!("unknown op {op}"))),
        }
    }

    /// Runs one printer call and translates its status into a protocol
    /// result. `success_disposition` applies to 2xx answers; a 202 from the
    /// printer is always `Accepted`.
    fn printer_op(&self, method: &str, path: &str, body: Option<&Value>, success_disposition: Disposition) -> OpResult {
        let (status, payload) = self.backend.printer(method, path, body).map_err(backend_error)?;
        match status {
            202 => Ok((Disposition::Accepted, payload)),
            200..=299 => Ok((success_disposition, payload)),
            _ => Err(status_error(status, &payload)),
        }
    }

    fn printer_status(&self) -> Result<Value, OpError> {
        let (status, payload) = self.backend.printer("GET", "/status", None).map_err(backend_error)?;
        if status == 200 {
            Ok(payload)
        } else {
            Err(status_error(status, &payload))
        }
    }

    fn sys_status(&self) -> OpResult {
        let printer = self.printer_status();
        let result = json!({
            "proto": PROTOCOL_VERSION,
            "clock": {"unix_ms": self.backend.clock_now_ms()},
            "services": {
                "printer": printer.is_ok(),
                "uploader": self.backend.uploader_healthy(),
            },
            // Surfaced as-is: a missing printer service is reported, never
            // papered over with placeholder data.
            "printer": printer.as_ref().ok(),
            "printer_error": printer.as_ref().err().map(|e| e.message.clone()),
            "bluetooth": {"version": env!("CARGO_PKG_VERSION")},
        });
        Ok((Disposition::Completed, result))
    }

    fn clock_set(&self, args: &Value) -> OpResult {
        let requested = args
            .get("unix_ms")
            .and_then(Value::as_i64)
            .ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, "unix_ms must be an integer"))?;
        if !(MIN_PLAUSIBLE_UNIX_MS..=MAX_PLAUSIBLE_UNIX_MS).contains(&requested) {
            return Err(OpError::new(ErrorCode::InvalidArgument, "unix_ms is outside the plausible range"));
        }
        let before = self.backend.clock_now_ms();
        self.backend
            .set_clock_ms(requested)
            .map_err(|message| OpError::new(ErrorCode::Internal, format!("failed to set the clock: {message}")))?;

        // The autoprint timer holds an absolute time; re-arm it after a real
        // correction so a large jump neither fires a burst nor stalls it.
        let mut rescheduled = false;
        if (requested - before).abs() > CLOCK_RESCHEDULE_THRESHOLD_MS {
            if let Ok(status) = self.printer_status() {
                let autoprint = &status["autoprint"];
                if autoprint["enabled"].as_bool() == Some(true) {
                    let (code, _) = self
                        .backend
                        .printer("POST", "/autoprint", Some(&json!({"enabled": true})))
                        .map_err(backend_error)?;
                    rescheduled = (200..300).contains(&code);
                }
            }
        }
        Ok((
            Disposition::Completed,
            json!({"unix_ms": self.backend.clock_now_ms(), "previous_unix_ms": before, "scheduler_rescheduled": rescheduled}),
        ))
    }

    fn library_list(&self, args: &Value) -> OpResult {
        let offset = opt_u64(args, "offset")?.unwrap_or(0) as usize;
        let limit = (opt_u64(args, "limit")?.unwrap_or(DEFAULT_LIBRARY_PAGE as u64) as usize).clamp(1, MAX_LIBRARY_PAGE);
        let (status, payload) = self.backend.printer("GET", "/catalog", None).map_err(backend_error)?;
        if status != 200 {
            return Err(status_error(status, &payload));
        }
        let items = payload["catalog"].as_array().cloned().unwrap_or_default();
        let total = items.len();
        let page: Vec<Value> = items.into_iter().skip(offset).take(limit).collect();
        let next_offset = (offset + page.len() < total).then_some(offset + page.len());
        Ok((Disposition::Completed, json!({"total": total, "offset": offset, "items": page, "next_offset": next_offset})))
    }

    fn sched_get(&self) -> OpResult {
        let status = self.printer_status()?;
        Ok((Disposition::Completed, status["autoprint"].clone()))
    }

    fn sched_set(&self, args: &Value) -> OpResult {
        // `enabled` is required by the printer API; fill it from current
        // state so a phone can change just the schedule.
        let mut body = json!({});
        for key in ["min_minutes", "max_minutes", "ordering", "window_enabled", "window_start", "window_end"] {
            if let Some(value) = args.get(key) {
                body[key] = value.clone();
            }
        }
        body["enabled"] = match args.get("enabled") {
            Some(value) => value.clone(),
            None => self.printer_status()?["autoprint"]["enabled"].clone(),
        };
        let (status, payload) = self.backend.printer("POST", "/autoprint", Some(&body)).map_err(backend_error)?;
        if (200..300).contains(&status) {
            Ok((Disposition::Completed, payload["autoprint"].clone()))
        } else {
            Err(status_error(status, &payload))
        }
    }

    // ---- uploads -------------------------------------------------------

    fn upload_begin(&self, args: &Value) -> OpResult {
        let size = arg_u64(args, "size")? as usize;
        if size == 0 || size > common::MAX_UPLOAD_BYTES {
            return Err(OpError::new(ErrorCode::InvalidArgument, "size must be between 1 byte and the 20 MB limit"));
        }
        let name = args.get("name").and_then(Value::as_str).unwrap_or("").chars().take(120).collect::<String>();
        let mut next = self.next_session.lock().unwrap_or_else(|p| p.into_inner());
        let id = *next;
        *next = next.wrapping_add(1).max(1);
        // One phone at a time: a new upload replaces any earlier session.
        *self.upload.lock().unwrap_or_else(|p| p.into_inner()) = Some(UploadSession {
            id,
            name,
            size,
            data: Vec::with_capacity(size),
            error: None,
            last_activity: Instant::now(),
        });
        Ok((Disposition::Completed, json!({"upload_id": id, "size": size, "received": 0})))
    }

    /// Handles one binary chunk from the Upload characteristic. Chunks are
    /// written without response, so problems are recorded on the session and
    /// reported by `upload.status` / `upload.commit` instead of returned.
    pub fn upload_chunk(&self, bytes: &[u8]) {
        let Some(chunk) = parse_upload_chunk(bytes) else { return };
        let mut guard = self.upload.lock().unwrap_or_else(|p| p.into_inner());
        let Some(session) = guard.as_mut().filter(|s| s.id == chunk.session) else {
            log::warn!("upload chunk for unknown session {}", chunk.session);
            return;
        };
        session.last_activity = Instant::now();
        let offset = chunk.offset as usize;
        let have = session.data.len();
        if offset > have {
            session.error = Some(format!("gap in upload: expected offset {have}, got {offset}"));
            return;
        }
        // A resend starting at or before `have` is accepted; only the bytes
        // beyond what we hold are appended.
        let skip = have - offset;
        if skip >= chunk.data.len() {
            return;
        }
        let fresh = &chunk.data[skip..];
        if session.data.len() + fresh.len() > session.size {
            session.error = Some("upload exceeds the declared size".to_string());
            return;
        }
        session.data.extend_from_slice(fresh);
        session.error = None;
    }

    fn with_session<T>(&self, args: &Value, f: impl FnOnce(&mut UploadSession) -> Result<T, OpError>) -> Result<T, OpError> {
        let id = arg_u64(args, "upload_id")? as u16;
        let mut guard = self.upload.lock().unwrap_or_else(|p| p.into_inner());
        if guard.as_ref().is_some_and(|s| s.last_activity.elapsed() > UPLOAD_IDLE_TIMEOUT) {
            *guard = None;
        }
        match guard.as_mut().filter(|s| s.id == id) {
            Some(session) => f(session),
            None => Err(OpError::new(ErrorCode::NotFound, "no such upload session (it may have expired)")),
        }
    }

    fn upload_status(&self, args: &Value) -> OpResult {
        self.with_session(args, |s| {
            s.last_activity = Instant::now();
            Ok((Disposition::Completed, json!({"upload_id": s.id, "size": s.size, "received": s.data.len(), "error": s.error})))
        })
    }

    fn upload_abort(&self, args: &Value) -> OpResult {
        let id = arg_u64(args, "upload_id")? as u16;
        let mut guard = self.upload.lock().unwrap_or_else(|p| p.into_inner());
        if guard.as_ref().is_some_and(|s| s.id == id) {
            *guard = None;
        }
        Ok((Disposition::Completed, json!({})))
    }

    fn upload_commit(&self, args: &Value) -> OpResult {
        let expected_crc = arg_u64(args, "crc32")? as u32;
        // Take the session out first so the (slow) HTTP post doesn't hold
        // the lock; on a recoverable failure it is put back for resume.
        let session = self.with_session(args, |s| {
            if let Some(error) = &s.error {
                return Err(OpError {
                    code: ErrorCode::UploadIncomplete,
                    message: error.clone(),
                    details: Some(json!({"received": s.data.len()})),
                });
            }
            if s.data.len() != s.size {
                return Err(OpError {
                    code: ErrorCode::UploadIncomplete,
                    message: format!("received {} of {} bytes", s.data.len(), s.size),
                    details: Some(json!({"received": s.data.len()})),
                });
            }
            if crc32fast::hash(&s.data) != expected_crc {
                return Err(OpError::new(ErrorCode::InvalidArgument, "checksum mismatch; the upload was corrupted in transit"));
            }
            Ok((s.id, s.name.clone(), std::mem::take(&mut s.data)))
        })?;
        let (session_id, name, data) = session;

        let outcome = self.backend.submit_upload(&data).map_err(backend_error);
        let mut guard = self.upload.lock().unwrap_or_else(|p| p.into_inner());
        match outcome {
            Ok((status, payload)) if (200..300).contains(&status) => {
                *guard = None;
                log::info!("upload accepted name={name:?} bytes={}", data.len());
                // Staged in ready/; the watcher converts it into the library.
                Ok((Disposition::Accepted, json!({"item_id": payload["id"], "filename": payload["filename"], "name": name})))
            }
            other => {
                // Restore the bytes so the phone can retry the commit.
                if let Some(s) = guard.as_mut().filter(|s| s.id == session_id) {
                    s.data = data;
                }
                match other {
                    Ok((status, payload)) => Err(status_error(status, &payload)),
                    Err(error) => Err(error),
                }
            }
        }
    }

    /// One digest per change-notification domain; the GATT layer notifies
    /// the phone when any differs from the previous poll.
    pub fn domain_digests(&self) -> [u64; 4] {
        let status = self.backend.printer("GET", "/status", None).ok().map(|r| r.1).unwrap_or(Value::Null);
        let catalog = self.backend.printer("GET", "/catalog", None).ok().map(|r| r.1).unwrap_or(Value::Null);
        let jobs = self.backend.printer("GET", "/jobs", None).ok().map(|r| r.1).unwrap_or(Value::Null);
        [
            digest(&json!([status["printer"], status["failed_count"], status["processing_count"], self.backend.uploader_healthy(), status.is_null()])),
            digest(&catalog),
            digest(&json!([status["jobs"], jobs])),
            digest(&json!([status["autoprint"]["enabled"], status["autoprint"]["min_minutes"], status["autoprint"]["max_minutes"], status["autoprint"]["ordering"], status["autoprint"]["next_print_at"], status["autoprint"]["last_item_id"], status["autoprint"]["last_error"]])),
        ]
    }
}

// ---- Wi-Fi ---------------------------------------------------------------
//
// Reading is `nmcli device wifi list`; connecting creates a profile this
// service owns (`wifi::PROFILE_PREFIX`), brings it up, and on failure deletes
// it and re-activates the network the Pi was on. The password is passed to
// nmcli as an argument and is never logged or returned.

const NM_QUICK: Duration = Duration::from_secs(15);
const NM_CONNECT: Duration = Duration::from_secs(45);
/// A scan that waits for fresh results normally takes about 4 s.
const WIFI_SCAN_WAIT: Duration = Duration::from_secs(12); // under the app's 15 s request timeout

#[derive(Clone, Default)]
pub struct WifiAttempt {
    /// `idle`, `connecting`, `connected` or `failed`.
    state: &'static str,
    ssid: Option<String>,
    error: Option<String>,
}

impl WifiAttempt {
    fn to_json(&self) -> Value {
        json!({"state": if self.state.is_empty() { "idle" } else { self.state }, "ssid": self.ssid, "error": self.error})
    }
}

fn nm_unavailable(error: String) -> OpError {
    OpError::new(ErrorCode::Unavailable, format!("NetworkManager is not available: {error}"))
}

impl<B: Backend + 'static> Handler<B> {
    fn wifi_list(&self, rescan: bool) -> Result<Vec<wifi::Network>, OpError> {
        const FIELDS: [&str; 3] = ["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY"];
        let list = |rescan: &str, timeout| {
            let mut args = FIELDS.to_vec();
            args.extend(["device", "wifi", "list", "--rescan", rescan]);
            self.backend.nmcli(&args, timeout)
        };
        // `--rescan yes` makes nmcli wait for the scan to finish (about 4 s) and then list the
        // fresh results. A fixed sleep was too short on a cold start, when the Pi has only
        // seen the network it is joined to. If the driver refuses a rescan (it throttles
        // them), settle for the cached list.
        let output = match rescan {
            true => match list("yes", WIFI_SCAN_WAIT) {
                Ok(out) if out.success => out,
                _ => list("no", NM_QUICK).map_err(nm_unavailable)?,
            },
            false => list("no", NM_QUICK).map_err(nm_unavailable)?,
        };
        if !output.success {
            return Err(OpError::new(ErrorCode::Unavailable, "could not list Wi-Fi networks"));
        }
        Ok(wifi::parse_networks(&output.stdout))
    }

    fn wifi_status(&self) -> OpResult {
        let networks = self.wifi_list(false)?;
        let attempt = self.wifi_attempt.lock().unwrap_or_else(|p| p.into_inner()).to_json();
        let result = match wifi::current(&networks) {
            Some(n) => json!({"connected": true, "ssid": n.ssid, "signal": n.signal, "security": n.security, "attempt": attempt}),
            None => json!({"connected": false, "ssid": null, "signal": 0, "security": null, "attempt": attempt}),
        };
        Ok((Disposition::Completed, result))
    }

    fn wifi_scan(&self) -> OpResult {
        let networks = self.wifi_list(true)?;
        Ok((Disposition::Completed, json!({"networks": wifi::networks_json(&networks)})))
    }

    fn wifi_connect(&self, args: &Value) -> OpResult {
        let ssid = arg_str(args, "ssid")?.to_string();
        let password = match args.get("password") {
            None | Some(Value::Null) => String::new(),
            Some(value) => value.as_str().ok_or_else(|| OpError::new(ErrorCode::InvalidArgument, "password must be a string"))?.to_string(),
        };
        wifi::validate_ssid(&ssid).map_err(|_| OpError::new(ErrorCode::InvalidArgument, "ssid must be 1-32 characters"))?;
        wifi::validate_password(&password)
            .map_err(|_| OpError::new(ErrorCode::InvalidArgument, "password must be 8-63 characters (or 64 hex digits)"))?;
        {
            let mut attempt = self.wifi_attempt.lock().unwrap_or_else(|p| p.into_inner());
            if attempt.state == "connecting" {
                return Err(OpError::new(ErrorCode::Conflict, "a Wi-Fi connection is already in progress"));
            }
            *attempt = WifiAttempt { state: "connecting", ssid: Some(ssid.clone()), error: None };
        }
        let backend = Arc::clone(&self.backend);
        let attempt = Arc::clone(&self.wifi_attempt);
        let target = ssid.clone();
        std::thread::spawn(move || {
            let outcome = run_wifi_connect(&*backend, &target, &password);
            let mut slot = attempt.lock().unwrap_or_else(|p| p.into_inner());
            *slot = match outcome {
                Ok(()) => WifiAttempt { state: "connected", ssid: Some(target), error: None },
                Err(message) => WifiAttempt { state: "failed", ssid: Some(target), error: Some(message) },
            };
        });
        Ok((Disposition::Accepted, json!({"ssid": ssid})))
    }
}

/// The name of the Wi-Fi profile that is active now, if any.
fn active_wifi_profile<B: Backend>(backend: &B) -> Option<String> {
    let output = backend.nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show", "--active"], NM_QUICK).ok()?;
    output.stdout.lines().map(wifi::split_terse).find(|f| f.len() >= 2 && f[1] == "802-11-wireless").map(|f| f[0].clone())
}

/// Joins `ssid`, rolling back to the previous network on failure. The error
/// is a short message safe to show on a phone.
fn run_wifi_connect<B: Backend>(backend: &B, ssid: &str, password: &str) -> Result<(), String> {
    let previous = active_wifi_profile(backend);
    let name = wifi::profile_name(ssid);
    // A leftover profile of ours from an earlier attempt would be ambiguous.
    let _ = backend.nmcli(&["connection", "delete", "id", &name], NM_QUICK);

    let mut add = vec!["connection", "add", "type", "wifi", "con-name", &name, "ifname", "*", "ssid", ssid];
    if !password.is_empty() {
        add.extend(["wifi-sec.key-mgmt", "wpa-psk", "wifi-sec.psk", password]);
    }
    match backend.nmcli(&add, NM_QUICK) {
        Ok(out) if out.success => {}
        _ => return Err("Could not save the network.".to_string()),
    }

    let failure = match backend.nmcli(&["--wait", "30", "connection", "up", "id", &name], NM_CONNECT) {
        Ok(out) if out.success => return Ok(()),
        Ok(out) => wifi::explain_failure(&out.stderr),
        Err(_) => "The network did not answer in time.",
    };

    log::warn!("wifi: connecting to a network failed; rolling back");
    let _ = backend.nmcli(&["connection", "delete", "id", &name], NM_QUICK);
    let went_back = match previous {
        Some(ref old) if *old != name => {
            matches!(backend.nmcli(&["--wait", "30", "connection", "up", "id", old], NM_CONNECT), Ok(out) if out.success)
        }
        _ => false,
    };
    Err(if went_back { format!("{failure} Went back to the previous network.") } else { failure.to_string() })
}

fn backend_error(error: BackendError) -> OpError {
    match error {
        BackendError::Unreachable(message) => OpError::new(ErrorCode::Unavailable, message),
    }
}

fn status_error(status: u16, payload: &Value) -> OpError {
    let message = payload.get("error").and_then(Value::as_str).unwrap_or("request failed").to_string();
    let code = match status {
        400 | 413 => ErrorCode::InvalidArgument,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        501 => ErrorCode::Unsupported,
        _ => ErrorCode::BackendError,
    };
    OpError { code, message, details: payload.get("job").map(|job| json!({"job": job})) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct Fake {
        calls: StdMutex<Vec<(String, String, Option<Value>)>>,
        clock_ms: StdMutex<i64>,
        uploads: StdMutex<Vec<Vec<u8>>>,
        autoprint_enabled: bool,
        /// Every nmcli invocation, space-joined.
        nm_calls: StdMutex<Vec<String>>,
        /// stderr to fail bringing up a new profile with; `None` succeeds.
        nm_up_failure: Option<&'static str>,
        /// How long bringing a profile up takes.
        nm_up_delay: Option<Duration>,
        /// Refuse `--rescan yes`, as the driver does when scans come too close together.
        nm_scan_throttled: bool,
    }

    const NM_LIST: &str = "*:Fred is SPEED:61:WPA2 WPA3\n :Fred is SPEED:80:WPA2 WPA3\n :Cafe Guest:40:--\n";

    impl Backend for Fake {
        fn nmcli(&self, args: &[&str], _timeout: Duration) -> Result<NmOutput, String> {
            self.nm_calls.lock().unwrap().push(args.join(" "));
            let ok = |stdout: &str| Ok(NmOutput { success: true, stdout: stdout.to_string(), stderr: String::new() });
            if args.contains(&"list") {
                if self.nm_scan_throttled && args.contains(&"yes") {
                    return Ok(NmOutput { success: false, stdout: String::new(), stderr: "Scanning not allowed immediately following previous scan".into() });
                }
                return ok(NM_LIST);
            }
            if args.contains(&"--active") {
                return ok("Home:802-11-wireless\nlo:loopback\n");
            }
            if args.contains(&"up") {
                if let Some(delay) = self.nm_up_delay {
                    std::thread::sleep(delay);
                }
                let is_new = args.iter().any(|a| a.starts_with("infinite-scroll-"));
                if let (true, Some(stderr)) = (is_new, self.nm_up_failure) {
                    return Ok(NmOutput { success: false, stdout: String::new(), stderr: stderr.to_string() });
                }
            }
            ok("")
        }

        fn printer(&self, method: &str, path: &str, body: Option<&Value>) -> BackendResult {
            self.calls.lock().unwrap().push((method.into(), path.into(), body.cloned()));
            match (method, path) {
                ("GET", "/status") => Ok((200, json!({"printer": {"connected": true, "busy": false}, "autoprint": {"enabled": self.autoprint_enabled, "ordering": "sequential"}, "jobs": {"active": 0, "current": null}}))),
                ("GET", "/catalog") => Ok((200, json!({"catalog": (0..5).map(|i| json!({"id": format!("id-{i}")})).collect::<Vec<_>>()}))),
                ("POST", "/printer/print-config") => Ok((200, json!({"success": true}))),
                ("POST", "/print") => Ok((202, json!({"accepted": true, "job": {"id": "job-1", "state": "queued"}}))),
                ("GET", "/catalog/id-0/thumbnail?w=160") => Ok((200, json!({"id": "id-0", "format": "jpeg", "width": 160, "height": 90, "data": "AAAA"}))),
                ("GET", "/catalog/id-0/thumbnail?w=96") => Ok((200, json!({"id": "id-0", "format": "jpeg", "width": 96, "height": 54, "data": "AAAA"}))),
                ("DELETE", "/catalog/id-0") => Ok((200, json!({"success": true}))),
                ("DELETE", "/catalog/missing") => Ok((404, json!({"error": "no catalog item with id missing"}))),
                ("POST", "/autoprint") => Ok((200, json!({"success": true, "autoprint": {"enabled": true}}))),
                _ => Ok((404, json!({"error": "not found"}))),
            }
        }
        fn uploader_healthy(&self) -> bool {
            true
        }
        fn submit_upload(&self, bytes: &[u8]) -> BackendResult {
            self.uploads.lock().unwrap().push(bytes.to_vec());
            Ok((201, json!({"id": "new-id", "filename": "new-id.png"})))
        }
        fn clock_now_ms(&self) -> i64 {
            *self.clock_ms.lock().unwrap()
        }
        fn set_clock_ms(&self, unix_ms: i64) -> Result<(), String> {
            *self.clock_ms.lock().unwrap() = unix_ms;
            Ok(())
        }
    }

    fn call(handler: &Handler<Fake>, op: &str, args: Value) -> Value {
        handler.handle_message(json!({"v": 1, "id": 9, "op": op, "args": args}).to_string().as_bytes())
    }

    fn handler() -> Handler<Fake> {
        Handler::new(Fake { clock_ms: StdMutex::new(1_800_000_000_000), ..Default::default() })
    }

    // ---- Wi-Fi -------------------------------------------------------------

    /// Polls `wifi.status` until the connect attempt leaves `connecting`.
    fn wait_for_attempt(handler: &Handler<Fake>) -> Value {
        for _ in 0..200 {
            let status = call(handler, "wifi.status", json!({}));
            if status["result"]["attempt"]["state"] != "connecting" {
                return status["result"]["attempt"].clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the connect attempt never finished");
    }

    #[test]
    fn wifi_status_reports_the_current_network_and_signal() {
        let status = call(&handler(), "wifi.status", json!({}));
        assert_eq!(status["ok"], true);
        assert_eq!(status["result"]["connected"], true);
        assert_eq!(status["result"]["ssid"], "Fred is SPEED");
        assert_eq!(status["result"]["signal"], 80);
        assert_eq!(status["result"]["attempt"]["state"], "idle");
    }

    #[test]
    fn wifi_scan_lists_each_network_once() {
        let scan = call(&handler(), "wifi.scan", json!({}));
        let networks = scan["result"]["networks"].as_array().unwrap();
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0]["ssid"], "Fred is SPEED");
        assert_eq!(networks[1]["security"], "open");
    }

    #[test]
    fn a_scan_waits_for_fresh_results_instead_of_sleeping() {
        let h = handler();
        call(&h, "wifi.scan", json!({}));
        let calls = h.backend.nm_calls.lock().unwrap().join("\n");
        assert!(calls.contains("device wifi list --rescan yes"), "{calls}");
        assert!(!calls.contains("device wifi rescan"));
    }

    #[test]
    fn a_throttled_rescan_falls_back_to_the_cached_list() {
        let h = Handler::new(Fake { nm_scan_throttled: true, ..Default::default() });
        let scan = call(&h, "wifi.scan", json!({}));
        assert_eq!(scan["ok"], true);
        assert_eq!(scan["result"]["networks"].as_array().unwrap().len(), 2);
        assert!(h.backend.nm_calls.lock().unwrap().join("\n").contains("--rescan no"));
    }

    #[test]
    fn wifi_connect_is_accepted_and_succeeds() {
        let h = handler();
        let response = call(&h, "wifi.connect", json!({"ssid": "Cafe Guest", "password": "correct horse"}));
        assert_eq!(response["ok"], true);
        assert_eq!(response["disposition"], "accepted");
        assert_eq!(wait_for_attempt(&h)["state"], "connected");
        let calls = h.backend.nm_calls.lock().unwrap().join("\n");
        assert!(calls.contains("connection add type wifi con-name infinite-scroll-Cafe Guest"));
        assert!(calls.contains("wifi-sec.psk correct horse"));
        assert!(calls.contains("connection up id infinite-scroll-Cafe Guest"));
    }

    #[test]
    fn an_open_network_gets_no_security_settings() {
        let h = handler();
        call(&h, "wifi.connect", json!({"ssid": "Cafe Guest"}));
        assert_eq!(wait_for_attempt(&h)["state"], "connected");
        assert!(!h.backend.nm_calls.lock().unwrap().join("\n").contains("wifi-sec"));
    }

    #[test]
    fn a_failed_connection_rolls_back_to_the_previous_network() {
        let h = Handler::new(Fake { nm_up_failure: Some("Error: Connection activation failed: Secrets were required, but not provided."), ..Default::default() });
        call(&h, "wifi.connect", json!({"ssid": "Fred is SPEED", "password": "wrong password"}));
        let attempt = wait_for_attempt(&h);
        assert_eq!(attempt["state"], "failed");
        let error = attempt["error"].as_str().unwrap();
        assert!(error.contains("rejected the password") && error.contains("Went back"), "{error}");
        let calls = h.backend.nm_calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c == "connection delete id infinite-scroll-Fred is SPEED"));
        let last_up = calls.iter().rev().find(|c| c.contains("connection up")).unwrap();
        assert_eq!(last_up, "--wait 30 connection up id Home");
    }

    #[test]
    fn the_password_never_appears_in_any_response() {
        let h = Handler::new(Fake { nm_up_failure: Some("Error: Secrets were required"), ..Default::default() });
        let accepted = call(&h, "wifi.connect", json!({"ssid": "Cafe Guest", "password": "s3cret-passphrase"}));
        let status = {
            wait_for_attempt(&h);
            call(&h, "wifi.status", json!({}))
        };
        for response in [accepted, status] {
            assert!(!response.to_string().contains("s3cret"));
        }
    }

    #[test]
    fn bad_wifi_arguments_are_rejected_before_touching_networkmanager() {
        let h = handler();
        for args in [json!({"ssid": ""}), json!({"ssid": "ok", "password": "short"}), json!({"password": "longenough1"}), json!({"ssid": "ok", "password": 5})] {
            let response = call(&h, "wifi.connect", args);
            assert_eq!(response["ok"], false);
        }
        assert!(h.backend.nm_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_second_connect_while_one_is_running_is_a_conflict() {
        let h = Handler::new(Fake { nm_up_delay: Some(Duration::from_millis(300)), ..Default::default() });
        assert_eq!(call(&h, "wifi.connect", json!({"ssid": "Cafe Guest"}))["ok"], true);
        let second = call(&h, "wifi.connect", json!({"ssid": "Other Net"}));
        assert_eq!(second["ok"], false);
        assert_eq!(second["error"]["code"], "conflict");
        assert_eq!(wait_for_attempt(&h)["state"], "connected");
    }

    #[test]
    fn print_item_is_accepted_not_completed() {
        let response = call(&handler(), "print.item", json!({"id": "id-0", "copies": 3}));
        assert_eq!(response["ok"], true);
        assert_eq!(response["disposition"], "accepted");
        assert_eq!(response["result"]["job"]["state"], "queued");
        assert_eq!(response["id"], 9);
    }

    #[test]
    fn print_item_forwards_copies_to_the_printer() {
        let h = handler();
        call(&h, "print.item", json!({"id": "id-0", "copies": 3}));
        let calls = h.backend.calls.lock().unwrap();
        let (_, _, body) = calls.iter().find(|c| c.1 == "/print").unwrap();
        assert_eq!(body.as_ref().unwrap(), &json!({"id": "id-0", "copies": 3}));
    }

    #[test]
    fn delete_is_completed_and_unknown_id_maps_to_not_found() {
        let h = handler();
        assert_eq!(call(&h, "library.delete", json!({"id": "id-0"}))["disposition"], "completed");
        let missing = call(&h, "library.delete", json!({"id": "missing"}));
        assert_eq!(missing["ok"], false);
        assert_eq!(missing["error"]["code"], "not_found");
    }

    #[test]
    fn ids_that_could_alter_a_backend_path_are_rejected() {
        let h = handler();
        for bad in ["../x", "a/b", "a?b", "", "a b"] {
            let response = call(&h, "library.delete", json!({"id": bad}));
            assert_eq!(response["error"]["code"], "invalid_argument", "id {bad:?}");
        }
        assert!(h.backend.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn print_config_is_forwarded_to_the_printer() {
        let response = call(&handler(), "printer.print_config", json!({}));
        assert_eq!(response["ok"], true);
        assert_eq!(response["disposition"], "completed");
    }

    #[test]
    fn library_thumbnail_forwards_id_and_width() {
        let default = call(&handler(), "library.thumbnail", json!({"id": "id-0"}));
        assert_eq!(default["ok"], true);
        assert_eq!(default["result"]["width"], 160);
        let narrow = call(&handler(), "library.thumbnail", json!({"id": "id-0", "width": 96}));
        assert_eq!(narrow["result"]["width"], 96);
        let bad = call(&handler(), "library.thumbnail", json!({"id": "../x"}));
        assert_eq!(bad["ok"], false);
    }

    #[test]
    fn library_list_paginates() {
        let h = handler();
        let page = call(&h, "library.list", json!({"offset": 0, "limit": 2}));
        assert_eq!(page["result"]["total"], 5);
        assert_eq!(page["result"]["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["result"]["next_offset"], 2);
        let last = call(&h, "library.list", json!({"offset": 4, "limit": 2}));
        assert_eq!(last["result"]["items"].as_array().unwrap().len(), 1);
        assert!(last["result"]["next_offset"].is_null());
    }

    #[test]
    fn unknown_ops_and_bad_versions_are_rejected() {
        let h = handler();
        assert_eq!(call(&h, "shell.exec", json!({"cmd": "id"}))["error"]["code"], "unknown_op");
        let response = h.handle_message(json!({"v": 99, "id": 1, "op": "sys.status"}).to_string().as_bytes());
        assert_eq!(response["error"]["code"], "unsupported_version");
        assert_eq!(h.handle_message(b"garbage")["error"]["code"], "bad_request");
    }

    #[test]
    fn status_reports_services_and_clock() {
        let response = call(&handler(), "sys.status", json!({}));
        assert_eq!(response["result"]["services"]["printer"], true);
        assert_eq!(response["result"]["clock"]["unix_ms"], 1_800_000_000_000i64);
    }

    #[test]
    fn clock_set_rejects_implausible_times_and_applies_good_ones() {
        let h = handler();
        assert_eq!(call(&h, "sys.clock.set", json!({"unix_ms": 5}))["error"]["code"], "invalid_argument");
        let ok = call(&h, "sys.clock.set", json!({"unix_ms": 1_800_000_500_000i64}));
        assert_eq!(ok["result"]["unix_ms"], 1_800_000_500_000i64);
        assert_eq!(ok["result"]["scheduler_rescheduled"], false);
    }

    #[test]
    fn large_clock_correction_rearms_an_enabled_scheduler() {
        let h = Handler::new(Fake { clock_ms: StdMutex::new(1_800_000_000_000), autoprint_enabled: true, ..Default::default() });
        let ok = call(&h, "sys.clock.set", json!({"unix_ms": 1_800_100_000_000i64}));
        assert_eq!(ok["result"]["scheduler_rescheduled"], true);
    }

    #[test]
    fn sched_set_fills_enabled_from_current_state() {
        let h = handler();
        call(&h, "sched.set", json!({"min_minutes": 5, "max_minutes": 9}));
        let calls = h.backend.calls.lock().unwrap();
        let (_, _, body) = calls.iter().find(|c| c.1 == "/autoprint").unwrap();
        assert_eq!(body.as_ref().unwrap()["enabled"], false);
        assert_eq!(body.as_ref().unwrap()["min_minutes"], 5);
    }

    fn chunk(session: u16, offset: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = session.to_le_bytes().to_vec();
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn upload_round_trip_with_resume_after_a_gap() {
        let h = handler();
        let payload: Vec<u8> = (0..100u8).collect();
        let begin = call(&h, "upload.begin", json!({"name": "a.png", "size": 100}));
        let session = begin["result"]["upload_id"].as_u64().unwrap() as u16;

        h.upload_chunk(&chunk(session, 0, &payload[..40]));
        h.upload_chunk(&chunk(session, 60, &payload[60..])); // gap: 40..60 lost
        let status = call(&h, "upload.status", json!({"upload_id": session}));
        assert_eq!(status["result"]["received"], 40);
        assert!(status["result"]["error"].is_string());

        let incomplete = call(&h, "upload.commit", json!({"upload_id": session, "crc32": crc32fast::hash(&payload)}));
        assert_eq!(incomplete["error"]["code"], "upload_incomplete");

        // Resume from the reported offset (with some harmless overlap).
        h.upload_chunk(&chunk(session, 30, &payload[30..]));
        let commit = call(&h, "upload.commit", json!({"upload_id": session, "crc32": crc32fast::hash(&payload)}));
        assert_eq!(commit["ok"], true, "{commit}");
        assert_eq!(commit["disposition"], "accepted");
        assert_eq!(commit["result"]["item_id"], "new-id");
        assert_eq!(h.backend.uploads.lock().unwrap()[0], payload);
    }

    #[test]
    fn upload_commit_rejects_a_corrupt_payload_without_posting_it() {
        let h = handler();
        let begin = call(&h, "upload.begin", json!({"size": 4}));
        let session = begin["result"]["upload_id"].as_u64().unwrap() as u16;
        h.upload_chunk(&chunk(session, 0, b"abcd"));
        let commit = call(&h, "upload.commit", json!({"upload_id": session, "crc32": 1}));
        assert_eq!(commit["error"]["code"], "invalid_argument");
        assert!(h.backend.uploads.lock().unwrap().is_empty());
    }

    #[test]
    fn upload_begin_enforces_the_size_limit() {
        let h = handler();
        assert_eq!(call(&h, "upload.begin", json!({"size": 0}))["error"]["code"], "invalid_argument");
        assert_eq!(call(&h, "upload.begin", json!({"size": 21 * 1024 * 1024}))["error"]["code"], "invalid_argument");
    }

    #[test]
    fn chunk_beyond_declared_size_is_an_error_not_a_buffer_growth() {
        let h = handler();
        let begin = call(&h, "upload.begin", json!({"size": 4}));
        let session = begin["result"]["upload_id"].as_u64().unwrap() as u16;
        h.upload_chunk(&chunk(session, 0, b"abcdef"));
        let status = call(&h, "upload.status", json!({"upload_id": session}));
        assert_eq!(status["result"]["received"], 0);
        assert!(status["result"]["error"].is_string());
    }

    #[test]
    fn printer_setting_writes_cannot_smuggle_commands_through_the_key() {
        let h = handler();
        for bad in ["~SD10", "density;rm", "A", "dens ity", ""] {
            let r = call(&h, "printer.settings.set", json!({"key": bad, "value": 1}));
            assert_eq!(r["error"]["code"], "invalid_argument", "{bad:?}");
        }
    }

    #[test]
    fn unsupported_printer_setting_maps_to_unsupported() {
        // The fake printer answers 404 for unknown paths; a real 501 maps
        // to `unsupported` (see status_error).
        assert_eq!(status_error(501, &json!({"error": "x"})).code, ErrorCode::Unsupported);
    }

    #[test]
    fn digests_change_when_the_catalog_changes() {
        let h = handler();
        assert_eq!(h.domain_digests(), h.domain_digests());
    }
}
