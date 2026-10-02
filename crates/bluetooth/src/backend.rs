//! Production `Backend`: loopback HTTP to the existing services, plus the
//! system clock.

use std::time::Duration;

use serde_json::Value;

use crate::handler::{Backend, BackendError, BackendResult};

pub struct HttpBackend {
    pub printer_url: String,
    pub printer_token: String,
    pub uploader_url: String,
    pub uploader_token: String,
}

fn run(request: ureq::Request, body: Option<&Value>, raw: Option<&[u8]>, content_type: Option<&str>) -> BackendResult {
    let result = match (body, raw) {
        (Some(json), _) => request.send_json(json.clone()),
        (None, Some(bytes)) => request.set("Content-Type", content_type.unwrap_or("application/octet-stream")).send_bytes(bytes),
        (None, None) => request.call(),
    };
    match result {
        Ok(response) => {
            let status = response.status();
            Ok((status, response.into_json().unwrap_or(Value::Null)))
        }
        Err(ureq::Error::Status(code, response)) => Ok((code, response.into_json().unwrap_or(Value::Null))),
        Err(ureq::Error::Transport(transport)) => Err(BackendError::Unreachable(transport.to_string())),
    }
}

impl HttpBackend {
    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(timeout).build()
    }
}

impl Backend for HttpBackend {
    fn printer(&self, method: &str, path: &str, body: Option<&Value>) -> BackendResult {
        let request = Self::agent(Duration::from_secs(10))
            .request(method, &format!("{}{}", self.printer_url, path))
            .set("Authorization", &format!("Bearer {}", self.printer_token));
        run(request, body, None, None)
    }

    fn uploader_healthy(&self) -> bool {
        Self::agent(Duration::from_secs(3)).get(&format!("{}/health", self.uploader_url)).call().is_ok()
    }

    fn submit_upload(&self, bytes: &[u8]) -> BackendResult {
        // The uploader sniffs the real format from the bytes, so the
        // content type here is informational only.
        let request = Self::agent(Duration::from_secs(60))
            .post(&format!("{}/uploads", self.uploader_url))
            .set("Authorization", &format!("Bearer {}", self.uploader_token));
        run(request, None, Some(bytes), Some("application/octet-stream"))
    }

    fn clock_now_ms(&self) -> i64 {
        (common::now_unix_seconds() * 1000.0) as i64
    }

    fn set_clock_ms(&self, unix_ms: i64) -> Result<(), String> {
        let spec = libc::timespec { tv_sec: (unix_ms / 1000) as libc::time_t, tv_nsec: ((unix_ms % 1000) * 1_000_000) as libc::c_long };
        // Needs CAP_SYS_TIME, granted to this unit alone in btcontrol.service.
        if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &spec) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}
