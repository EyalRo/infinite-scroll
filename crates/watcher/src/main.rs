use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Config {
    ready_dir: PathBuf,
    complete_dir: PathBuf,
    failed_dir: PathBuf,
    poll_interval: Duration,
}

fn config_from_env() -> Config {
    Config {
        ready_dir: PathBuf::from(env::var("WATCHER_READY_DIR").expect("WATCHER_READY_DIR must be set")),
        complete_dir: PathBuf::from(env::var("WATCHER_COMPLETE_DIR").expect("WATCHER_COMPLETE_DIR must be set")),
        failed_dir: PathBuf::from(env::var("WATCHER_FAILED_DIR").expect("WATCHER_FAILED_DIR must be set")),
        poll_interval: Duration::from_secs(
            env::var("WATCHER_POLL_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(3),
        ),
    }
}

fn main() {
    let config = config_from_env();
    fs::create_dir_all(&config.ready_dir).expect("failed to create ready dir");
    fs::create_dir_all(&config.complete_dir).expect("failed to create complete dir");
    fs::create_dir_all(&config.failed_dir).expect("failed to create failed dir");
    eprintln!("watcher polling {:?} every {:?}", config.ready_dir, config.poll_interval);

    loop {
        if let Err(error) = tick(&config) {
            eprintln!("watcher tick failed: {error}");
        }
        std::thread::sleep(config.poll_interval);
    }
}

fn tick(config: &Config) -> std::io::Result<()> {
    for entry in fs::read_dir(&config.ready_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        process_one(config, &path);
    }
    Ok(())
}

fn process_one(config: &Config, path: &Path) {
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown").to_string();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("watcher: failed to read {path:?}: {error}");
            return;
        }
    };

    match common::normalize_and_convert(&bytes) {
        Ok(job) => {
            let zpl_path = config.complete_dir.join(format!("{id}.zpl"));
            let zpl_tmp_path = config.complete_dir.join(format!("{id}.zpl.tmp"));
            let sidecar_path = config.complete_dir.join(format!("{id}.json"));
            let sidecar_tmp_path = config.complete_dir.join(format!("{id}.json.tmp"));
            let sidecar = serde_json::json!({
                "id": id,
                "original_filename": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "added_at": common::now_unix_seconds(),
                "print_count": 0,
                "last_printed_at": null,
            });

            // Write to temp paths first
            if let Err(error) = fs::write(&zpl_tmp_path, job.text.as_bytes()) {
                eprintln!("watcher: failed to write temporary ZPL for {id}: {error}");
                move_to_failed(config, path, &id);
                return;
            }

            if let Err(error) = fs::write(&sidecar_tmp_path, serde_json::to_vec_pretty(&sidecar).unwrap()) {
                eprintln!("watcher: failed to write temporary sidecar for {id}: {error}");
                // Clean up the ZPL temp file if sidecar write fails
                if let Err(e) = fs::remove_file(&zpl_tmp_path) {
                    eprintln!("watcher: failed to clean up temporary ZPL for {id}: {e}");
                }
                move_to_failed(config, path, &id);
                return;
            }

            // Atomically rename both files to their final names
            if let Err(error) = fs::rename(&zpl_tmp_path, &zpl_path) {
                eprintln!("watcher: failed to finalize ZPL for {id}: {error}");
                // Clean up temp files
                let _ = fs::remove_file(&zpl_tmp_path);
                let _ = fs::remove_file(&sidecar_tmp_path);
                move_to_failed(config, path, &id);
                return;
            }

            if let Err(error) = fs::rename(&sidecar_tmp_path, &sidecar_path) {
                eprintln!("watcher: failed to finalize sidecar for {id}: {error}");
                // Clean up the ZPL file that was already renamed
                if let Err(e) = fs::remove_file(&zpl_path) {
                    eprintln!("watcher: failed to clean up ZPL file for {id}: {e}");
                }
                let _ = fs::remove_file(&sidecar_tmp_path);
                move_to_failed(config, path, &id);
                return;
            }

            // Success: remove the original source file
            if let Err(error) = fs::remove_file(path) {
                eprintln!("watcher: failed to remove source file for {id}: {error}");
            }
            eprintln!("watcher: converted {id} ({}x{})", job.width, job.height);
        }
        Err(error) => {
            eprintln!("watcher: conversion failed for {id}: {error}");
            move_to_failed(config, path, &id);
        }
    }
}

/// A file that can never be converted (corrupt upload, etc.) must not be
/// retried forever on every tick -- move it aside instead.
fn move_to_failed(config: &Config, path: &Path, id: &str) {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let destination = config.failed_dir.join(format!("{id}.{extension}"));
    if let Err(error) = fs::rename(path, destination) {
        eprintln!("watcher: failed to move {id} to failed dir: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("watcher-test-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds a `Config` pointed at fresh, uniquely-named temp subdirectories
    /// (ready/complete/failed) so tests can't interfere with each other or
    /// with a real watcher instance. Caller is responsible for
    /// `fs::remove_dir_all`-ing the returned base dir when done.
    fn test_config(label: &str) -> (Config, PathBuf) {
        let base = unique_temp_dir(label);
        let ready_dir = base.join("ready");
        let complete_dir = base.join("complete");
        let failed_dir = base.join("failed");
        fs::create_dir_all(&ready_dir).unwrap();
        fs::create_dir_all(&complete_dir).unwrap();
        fs::create_dir_all(&failed_dir).unwrap();
        let config = Config { ready_dir, complete_dir, failed_dir, poll_interval: Duration::from_secs(3) };
        (config, base)
    }

    // --- Minimal hand-rolled PNG encoder, used only by these tests. `watcher`
    // doesn't otherwise depend on the `image` crate (that's `common`'s job),
    // and pulling it in just to fabricate a test fixture isn't worth a new
    // dependency -- a few bytes of raw PNG/zlib/DEFLATE framing is simpler
    // and has no extra moving parts to trust.

    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        const MOD_ADLER: u32 = 65521;
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for &byte in data {
            a = (a + byte as u32) % MOD_ADLER;
            b = (b + a) % MOD_ADLER;
        }
        (b << 16) | a
    }

    fn png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut type_and_data = Vec::with_capacity(4 + data.len());
        type_and_data.extend_from_slice(chunk_type);
        type_and_data.extend_from_slice(data);
        out.extend_from_slice(&type_and_data);
        out.extend_from_slice(&crc32(&type_and_data).to_be_bytes());
    }

    /// A tiny, well-formed 8-bit grayscale PNG: real IHDR/IDAT/IEND chunks,
    /// a real zlib wrapper, and one or more DEFLATE "stored" (uncompressed)
    /// blocks -- a valid, if degenerate, DEFLATE stream that any conforming
    /// decoder (including the `image`/`png` crates `common` uses) accepts.
    fn tiny_valid_png(width: u32, height: u32, gray: u8) -> Vec<u8> {
        let mut raw = Vec::new();
        for _ in 0..height {
            raw.push(0u8); // filter type: none
            raw.extend(std::iter::repeat(gray).take(width as usize));
        }

        let mut zlib = vec![0x78, 0x01]; // zlib header: deflate, 32K window, valid checksum
        let mut remaining = raw.as_slice();
        loop {
            let chunk_len = remaining.len().min(65535);
            let (chunk, rest) = remaining.split_at(chunk_len);
            let is_final = rest.is_empty();
            zlib.push(if is_final { 1 } else { 0 }); // BFINAL | BTYPE=00 (stored)
            zlib.extend_from_slice(&(chunk_len as u16).to_le_bytes());
            zlib.extend_from_slice(&(!(chunk_len as u16)).to_le_bytes());
            zlib.extend_from_slice(chunk);
            remaining = rest;
            if is_final {
                break;
            }
        }
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8-bit depth, grayscale, deflate, filter 0, no interlace
        png_chunk(&mut png, b"IHDR", &ihdr);
        png_chunk(&mut png, b"IDAT", &zlib);
        png_chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn process_one_converts_a_valid_png_and_moves_it_into_complete() {
        let (config, base) = test_config("valid-png");
        let id = "abc123";
        let source_path = config.ready_dir.join(format!("{id}.png"));
        fs::write(&source_path, tiny_valid_png(4, 4, 10)).unwrap();

        process_one(&config, &source_path);

        assert!(!source_path.exists(), "converted source file should be removed from ready/");
        let zpl_path = config.complete_dir.join(format!("{id}.zpl"));
        let sidecar_path = config.complete_dir.join(format!("{id}.json"));
        assert!(zpl_path.exists(), "expected {zpl_path:?} to exist");
        assert!(sidecar_path.exists(), "expected {sidecar_path:?} to exist");

        let sidecar: serde_json::Value = serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
        assert_eq!(sidecar["id"], id);
        assert_eq!(sidecar["original_filename"], format!("{id}.png"));
        assert_eq!(sidecar["print_count"], 0);

        assert_eq!(fs::read_dir(&config.failed_dir).unwrap().count(), 0, "nothing should have landed in failed/");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn process_one_moves_a_corrupt_file_to_failed_without_crashing() {
        let (config, base) = test_config("corrupt");
        let id = "notanimage";
        let source_path = config.ready_dir.join(format!("{id}.png"));
        fs::write(&source_path, b"this is definitely not a PNG or JPEG").unwrap();

        process_one(&config, &source_path);

        assert!(!source_path.exists(), "corrupt file should be moved out of ready/");
        assert!(config.failed_dir.join(format!("{id}.png")).exists(), "corrupt file should land in failed/");
        assert_eq!(fs::read_dir(&config.complete_dir).unwrap().count(), 0, "nothing should have landed in complete/");

        let _ = fs::remove_dir_all(&base);
    }
}
