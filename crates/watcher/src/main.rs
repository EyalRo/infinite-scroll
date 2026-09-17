use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
                "added_at": now_unix_seconds(),
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

fn now_unix_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}
