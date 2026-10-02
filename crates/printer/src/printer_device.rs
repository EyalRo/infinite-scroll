use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub struct PrintResult {
    pub success: bool,
    pub message: String,
}

static PRINT_BUSY: AtomicBool = AtomicBool::new(false);

/// Darkness (0-30) sent with every job, or -1 for "leave the printer as it is".
/// Kept here, not only on the printer, so a printer power cycle can't silently
/// revert a setting the user chose.
static DARKNESS: AtomicI32 = AtomicI32::new(-1);

pub fn set_darkness(value: Option<u8>) {
    DARKNESS.store(value.map(i32::from).unwrap_or(-1), Ordering::SeqCst);
}

pub fn is_available(device_path: &Path) -> bool {
    std::fs::metadata(device_path).map(|meta| meta.file_type().is_char_device()).unwrap_or(false)
}

pub fn is_busy() -> bool {
    PRINT_BUSY.load(Ordering::SeqCst)
}

fn preflight(zpl_text: &str, device_path: &Path) -> Option<String> {
    if zpl_text.is_empty() {
        return Some("job is empty".to_string());
    }
    if !device_path.exists() {
        return Some(format!("printer device {} not present", device_path.display()));
    }
    if !is_available(device_path) {
        return Some(format!("{} is not a character device", device_path.display()));
    }
    None
}

/// Sends `command` and returns whatever the printer answers within `wait`
/// (the usblp return channel). Opens the device read/write non-blocking, so
/// a printer that never answers can't wedge a thread, and takes the same
/// busy flag as `print_zpl` so it never interleaves with a print.
pub fn query(device_path: &Path, command: &str, wait: Duration) -> Result<String, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(error) = preflight(command, device_path) {
        return Err(error);
    }
    if PRINT_BUSY.swap(true, Ordering::SeqCst) {
        return Err("printer busy: a previous print is still in progress".to_string());
    }
    let result = (|| {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(device_path)
            .map_err(|error| format!("open failed: {error}"))?;
        file.write_all(command.as_bytes()).and_then(|_| file.flush()).map_err(|error| format!("write failed: {error}"))?;
        let deadline = std::time::Instant::now() + wait;
        let mut reply = Vec::new();
        let mut last_data = None;
        let mut buffer = [0u8; 512];
        while std::time::Instant::now() < deadline {
            match file.read(&mut buffer) {
                Ok(0) => std::thread::sleep(Duration::from_millis(40)),
                Ok(n) => {
                    reply.extend_from_slice(&buffer[..n]);
                    last_data = Some(std::time::Instant::now());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // The report arrives in bursts; stop once it has gone quiet.
                    if last_data.is_some_and(|at| at.elapsed() > Duration::from_millis(600)) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(error) => return Err(format!("read failed: {error}")),
            }
        }
        Ok(String::from_utf8_lossy(&reply).replace('\0', ""))
    })();
    PRINT_BUSY.store(false, Ordering::SeqCst);
    result
}

/// Serialized (one write at a time) and bounded (default 15s) -- this
/// exists because of a 2026-09-13 incident in an earlier implementation,
/// where a stuck/offline printer left a raw device write hanging
/// indefinitely and wedged the whole process; the bounded timeout plus the
/// `AtomicBool` serialization above are what prevent a repeat here.
pub fn print_zpl(device_path: &Path, zpl_text: &str, timeout: Duration) -> PrintResult {
    if let Some(error) = preflight(zpl_text, device_path) {
        return PrintResult { success: false, message: error };
    }

    if PRINT_BUSY.swap(true, Ordering::SeqCst) {
        return PrintResult {
            success: false,
            message: "printer busy: a previous print is still in progress".to_string(),
        };
    }

    let (sender, receiver) = mpsc::channel();
    let device_path = device_path.to_path_buf();
    let darkness = DARKNESS.load(Ordering::SeqCst);
    let zpl_text = if darkness >= 0 { format!("~SD{darkness:02}{zpl_text}") } else { zpl_text.to_string() };
    std::thread::spawn(move || {
        let outcome = OpenOptions::new()
            .write(true)
            .open(&device_path)
            .and_then(|mut file| file.write_all(zpl_text.as_bytes()).and_then(|_| file.flush()));
        PRINT_BUSY.store(false, Ordering::SeqCst);
        let _ = sender.send(outcome);
    });

    match receiver.recv_timeout(timeout) {
        Ok(Ok(())) => PrintResult { success: true, message: "printed".to_string() },
        Ok(Err(error)) => PrintResult { success: false, message: format!("write failed: {error}") },
        Err(_) => PrintResult {
            success: false,
            message: format!(
                "write to printer timed out after {:.0}s — check paper, cover, and power on the Arkscan",
                timeout.as_secs_f64()
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_flag_reads_false_before_any_print_starts() {
        assert!(!is_busy());
    }

    #[test]
    fn rejects_an_empty_job_before_touching_the_device() {
        let result = print_zpl(Path::new("/dev/null"), "", Duration::from_secs(1));
        assert!(!result.success);
        assert_eq!(result.message, "job is empty");
    }

    #[test]
    fn reports_a_missing_device_path() {
        let result = print_zpl(Path::new("/does/not/exist"), "^XA\n^XZ\n", Duration::from_secs(1));
        assert!(!result.success);
        assert!(result.message.contains("not present"));
    }

    #[test]
    fn reports_a_non_character_device_path() {
        let path = std::env::temp_dir().join(format!("printer-device-test-{}", std::process::id()));
        std::fs::write(&path, b"not a device").unwrap();
        let result = print_zpl(&path, "^XA\n^XZ\n", Duration::from_secs(1));
        assert!(!result.success);
        assert!(result.message.contains("not a character device"));
        let _ = std::fs::remove_file(&path);
    }
}
