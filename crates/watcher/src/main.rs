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
            let sidecar_path = config.complete_dir.join(format!("{id}.json"));
            let sidecar = serde_json::json!({
                "id": id,
                "original_filename": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "added_at": now_unix_seconds(),
                "print_count": 0,
                "last_printed_at": null,
            });
            let write_result = fs::write(&zpl_path, job.text.as_bytes())
                .and_then(|_| fs::write(&sidecar_path, serde_json::to_vec_pretty(&sidecar).unwrap()));
            match write_result {
                Ok(()) => {
                    let _ = fs::remove_file(path);
                    eprintln!("watcher: converted {id} ({}x{})", job.width, job.height);
                }
                Err(error) => {
                    eprintln!("watcher: failed to write converted output for {id}: {error}");
                    move_to_failed(config, path, &id);
                }
            }
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
    let _ = fs::rename(path, destination);
}

fn now_unix_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}
