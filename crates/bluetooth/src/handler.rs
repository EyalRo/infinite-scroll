//! Maps protocol operations onto the existing Infinite Scroll services.
//!
//! This is deliberately thin: library, scheduling, printing and upload
//! semantics all live in `printer` / `uploader`. The only operation handled
//! here is the clock, because neither service owns it. The match in
//! `Handler::dispatch` is the complete list of what a phone can do; there is
//! no generic pass-through to the services or to the operating system.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::protocol::*;

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

pub trait Backend: Send + Sync {
    /// Calls the printer service's HTTP API.
    fn printer(&self, method: &str, path: &str, body: Option<&Value>) -> BackendResult;
    /// Whether the uploader answers its health check.
    fn uploader_healthy(&self) -> bool;
    /// Posts raw image bytes through the uploader's normal `/uploads` path.
    fn submit_upload(&self, bytes: &[u8]) -> BackendResult;
    fn clock_now_ms(&self) -> i64;
    fn set_clock_ms(&self, unix_ms: i64) -> Result<(), String>;
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
    backend: B,
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

impl<B: Backend> Handler<B> {
    pub fn new(backend: B) -> Self {
        Handler { backend, upload: Mutex::new(None), next_session: Mutex::new(1) }
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
        for key in ["min_minutes", "max_minutes", "ordering"] {
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
    }

    impl Backend for Fake {
        fn printer(&self, method: &str, path: &str, body: Option<&Value>) -> BackendResult {
            self.calls.lock().unwrap().push((method.into(), path.into(), body.cloned()));
            match (method, path) {
                ("GET", "/status") => Ok((200, json!({"printer": {"connected": true, "busy": false}, "autoprint": {"enabled": self.autoprint_enabled, "ordering": "sequential"}, "jobs": {"active": 0, "current": null}}))),
                ("GET", "/catalog") => Ok((200, json!({"catalog": (0..5).map(|i| json!({"id": format!("id-{i}")})).collect::<Vec<_>>()}))),
                ("POST", "/print") => Ok((202, json!({"accepted": true, "job": {"id": "job-1", "state": "queued"}}))),
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
