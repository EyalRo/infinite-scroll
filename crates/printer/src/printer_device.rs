use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub struct PrintResult {
    pub success: bool,
    pub message: String,
}

static PRINT_BUSY: AtomicBool = AtomicBool::new(false);

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

/// Serialized (one write at a time) and bounded (default 15s) -- see
/// app/printer.py's module docstring for the 2026-09-13 incident this
/// exists to prevent: a stuck/offline printer must never hang the process
/// indefinitely.
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
    let zpl_text = zpl_text.to_string();
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
