mod library;
mod printer_device;
mod scheduler;
mod state;
mod upload_proxy;

use std::env;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::http::{header_value, json_response};
use tiny_http::{Method, Server};

const INDEX_HTML: &str = include_str!("../../../web/library/index.html");
const STYLE_CSS: &str = include_str!("../../../web/library/style.css");
const APP_JS: &str = include_str!("../../../web/library/app.js");

struct Config {
    token: String,
    state_path: PathBuf,
    complete_dir: PathBuf,
    failed_dir: PathBuf,
    device_path: PathBuf,
    bind_addr: String,
    state_lock: std::sync::Mutex<()>,
    uploader_addr: String,
    uploader_token: String,
}

fn config_from_env() -> Config {
    Config {
        token: env::var("PRINTER_TOKEN").expect("PRINTER_TOKEN must be set"),
        state_path: PathBuf::from(env::var("PRINTER_STATE_PATH").expect("PRINTER_STATE_PATH must be set")),
        complete_dir: PathBuf::from(env::var("PRINTER_COMPLETE_DIR").expect("PRINTER_COMPLETE_DIR must be set")),
        failed_dir: PathBuf::from(env::var("PRINTER_FAILED_DIR").expect("PRINTER_FAILED_DIR must be set")),
        device_path: PathBuf::from(env::var("PRINTER_DEVICE_PATH").unwrap_or_else(|_| "/dev/usb/lp0".to_string())),
        bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8082".to_string()),
        state_lock: std::sync::Mutex::new(()),
        // Lets printer proxy POST /uploads to the (genuinely separate)
        // uploader service over loopback -- see upload_proxy.rs for why.
        uploader_addr: env::var("UPLOADER_INTERNAL_ADDR").unwrap_or_else(|_| "127.0.0.1:8081".to_string()),
        uploader_token: env::var("UPLOADER_TOKEN").expect("UPLOADER_TOKEN must be set"),
    }
}

fn main() {
    let config = Arc::new(config_from_env());
    std::fs::create_dir_all(&config.complete_dir).expect("failed to create complete dir");
    if let Some(parent) = config.state_path.parent() {
        std::fs::create_dir_all(parent).expect("failed to create state dir");
    }

    let scheduler_config = Arc::clone(&config);
    std::thread::spawn(move || scheduler_loop(scheduler_config));

    let server = Server::http(&config.bind_addr).expect("failed to bind printer HTTP server");
    eprintln!("printer listening on {}", config.bind_addr);
    for mut request in server.incoming_requests() {
        let response = handle(&config, &mut request);
        let _ = request.respond(response);
    }
}

fn handle(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    if *request.method() == Method::Options {
        return common::http::cors_preflight_response();
    }
    let raw_url = request.url().to_string();
    let url = raw_url.split('?').next().unwrap_or(&raw_url).to_string();
    let method = request.method().clone();

    if url == "/health" && method == Method::Get {
        return common::http::with_cors(json_response(200, &serde_json::json!({"status": "ok"})));
    }
    if url == "/" && method == Method::Get {
        return common::http::with_cors(static_response(INDEX_HTML, "text/html; charset=utf-8"));
    }
    if url == "/style.css" && method == Method::Get {
        return common::http::with_cors(static_response(STYLE_CSS, "text/css; charset=utf-8"));
    }
    if url == "/app.js" && method == Method::Get {
        return common::http::with_cors(static_response(APP_JS, "application/javascript; charset=utf-8"));
    }

    let bearer = header_value(request, "Authorization");
    let access_jwt = header_value(request, "Cf-Access-Jwt-Assertion");
    if !common::auth::is_authorized(bearer, access_jwt, &config.token) {
        return common::http::with_cors(json_response(401, &serde_json::json!({"error": "unauthorized"})));
    }

    let response = match (url.as_str(), &method) {
        ("/status", Method::Get) => status_response(config),
        ("/catalog", Method::Get) => {
            let items = library::list(&config.complete_dir);
            json_response(200, &serde_json::json!({"catalog": items}))
        }
        (path, Method::Delete) if path.starts_with("/catalog/") => {
            let id = path.trim_start_matches("/catalog/");
            match library::remove(&config.complete_dir, id) {
                Some(item) => json_response(200, &serde_json::json!({"success": true, "removed": item})),
                None => json_response(404, &serde_json::json!({"error": format!("no catalog item with id {id}")})),
            }
        }
        (path, Method::Get) if path.starts_with("/catalog/") && path.ends_with("/preview.png") => {
            let id = path.trim_start_matches("/catalog/").trim_end_matches("/preview.png");
            return common::http::with_cors(preview_png_response(config, id));
        }
        (path, Method::Post) if path.starts_with("/catalog/") && path.ends_with("/print") => {
            let id = path.trim_start_matches("/catalog/").trim_end_matches("/print");
            manual_print_response(config, id)
        }
        ("/autoprint", Method::Post) => configure_autoprint(config, request),
        ("/uploads", Method::Post) => upload_proxy_response(config, request),
        _ => json_response(404, &serde_json::json!({"error": "not found"})),
    };
    common::http::with_cors(response)
}

/// Relays an upload to the uploader service over loopback -- see
/// upload_proxy.rs for why this exists as same-origin proxy rather than
/// having the browser call uploader's own (genuinely separate) origin
/// directly.
fn upload_proxy_response(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut bytes = Vec::new();
    if let Err(error) = request.as_reader().take(common::MAX_UPLOAD_BYTES as u64 + 1).read_to_end(&mut bytes) {
        return json_response(400, &serde_json::json!({"error": format!("failed to read request body: {error}")}));
    }
    if bytes.len() as u64 > common::MAX_UPLOAD_BYTES as u64 {
        return json_response(413, &serde_json::json!({"error": "upload exceeds the 20 MB limit"}));
    }
    match upload_proxy::forward_upload(&config.uploader_addr, &config.uploader_token, &bytes) {
        Ok((status, body)) => {
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header name/value is always valid");
            tiny_http::Response::from_data(body).with_status_code(status).with_header(header)
        }
        Err(error) => json_response(502, &serde_json::json!({"error": format!("upload proxy failed: {error}")})),
    }
}

/// Renders a catalog item's stored ZPL job back into the exact B&W bitmap
/// it will print, so the frontend's preview shows the real thing rather
/// than just a filename -- the original upload is discarded once
/// converted, so there's nothing else image-like to show.
fn preview_png_response(config: &Config, id: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    if !library::is_valid_id(id) {
        return json_response(404, &serde_json::json!({"error": "not found"}));
    }
    let Ok(zpl_text) = library::read_zpl(&config.complete_dir, id) else {
        return json_response(404, &serde_json::json!({"error": format!("no catalog item with id {id}")}));
    };
    match common::zpl_to_preview_png(&zpl_text) {
        Ok(png_bytes) => {
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..])
                .expect("static header name/value is always valid");
            tiny_http::Response::from_data(png_bytes).with_header(header)
        }
        Err(error) => json_response(500, &serde_json::json!({"error": format!("failed to render preview: {error}")})),
    }
}

/// Prints one catalog item immediately, on demand -- deliberately
/// independent of the autoprint scheduler: it never touches `Settings`
/// (`shuffle_queue`, `last_item_id`, `next_print_at`), only the item's own
/// print_count/last_printed_at, so a manual print neither consumes a slot
/// in nor perturbs the ordering of the current shuffle-cycle/sequential
/// pass. Safe to call concurrently with the scheduler's own prints --
/// `printer_device::print_zpl` already serializes physical writes via its
/// busy flag and just returns a clean "printer busy" error on collision.
fn manual_print_response(config: &Config, id: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    if !library::is_valid_id(id) {
        return json_response(404, &serde_json::json!({"error": "not found"}));
    }
    if library::get(&config.complete_dir, id).is_none() {
        return json_response(404, &serde_json::json!({"error": format!("no catalog item with id {id}")}));
    }
    let Ok(zpl_text) = library::read_zpl(&config.complete_dir, id) else {
        return json_response(404, &serde_json::json!({"error": format!("no print job stored for {id}")}));
    };
    let result = printer_device::print_zpl(&config.device_path, &zpl_text, Duration::from_secs(15));
    if result.success {
        let _ = library::mark_printed(&config.complete_dir, id, common::now_unix_seconds());
        json_response(200, &serde_json::json!({"success": true, "message": result.message}))
    } else {
        json_response(502, &serde_json::json!({"success": false, "error": result.message}))
    }
}

fn static_response(body: &'static str, content_type: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let content_type_header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
        .expect("static header name/value is always valid");
    // Without this, Cloudflare's edge applies its own default TTL to these
    // extensions (.html/.css/.js) and keeps serving a pre-deploy copy for
    // hours after a redeploy, since the binary embeds these via include_str!
    // and there's no per-deploy filename/hash to bust the cache key with.
    let cache_control_header = tiny_http::Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..])
        .expect("static header name/value is always valid");
    tiny_http::Response::from_data(body.as_bytes().to_vec())
        .with_header(content_type_header)
        .with_header(cache_control_header)
}

fn status_response(config: &Config) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let settings = state::load(&config.state_path);
    let items = library::list(&config.complete_dir);
    let failed_count = std::fs::read_dir(&config.failed_dir).map(|entries| entries.count()).unwrap_or(0);
    json_response(
        200,
        &serde_json::json!({
            "printer": {
                "connected": printer_device::is_available(&config.device_path),
                "busy": printer_device::is_busy(),
            },
            "catalog_count": items.len(),
            "failed_count": failed_count,
            "autoprint": {
                "enabled": settings.enabled,
                "min_minutes": settings.min_minutes,
                "max_minutes": settings.max_minutes,
                "ordering": settings.ordering,
                "next_print_at": settings.next_print_at,
                "last_item_id": settings.last_item_id,
                "last_error": settings.last_error,
            },
        }),
    )
}

fn configure_autoprint(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut body = String::new();
    if std::io::Read::read_to_string(request.as_reader(), &mut body).is_err() {
        return json_response(400, &serde_json::json!({"error": "failed to read request body"}));
    }
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&body) else {
        return json_response(400, &serde_json::json!({"error": "invalid JSON body"}));
    };
    let Some(enabled) = payload.get("enabled").and_then(|v| v.as_bool()) else {
        return json_response(400, &serde_json::json!({"error": "enabled must be a boolean"}));
    };

    // A poisoned lock still guards a plain JSON struct, not something that can be
    // corrupted by a panic mid-mutation in any way that matters here -- recovering
    // it is preferable to letting every subsequent .lock().unwrap() panic too,
    // which (since only the panicking thread dies, not the process) would freeze
    // the scheduler forever without systemd's Restart=always ever kicking in.
    let _guard = config.state_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut settings = state::load(&config.state_path);
    if let Some(value) = payload.get("min_minutes").and_then(|v| v.as_f64()) {
        settings.min_minutes = value;
    }
    if let Some(value) = payload.get("max_minutes").and_then(|v| v.as_f64()) {
        settings.max_minutes = value;
    }
    if let Some(ordering) = payload.get("ordering").and_then(|v| v.as_str()) {
        settings.ordering = match ordering {
            "random" => state::Ordering::Random,
            "sequential" => state::Ordering::Sequential,
            _ => return json_response(400, &serde_json::json!({"error": "ordering must be \"random\" or \"sequential\""})),
        };
    }
    if settings.min_minutes <= 0.0 || settings.max_minutes < settings.min_minutes || settings.max_minutes > 10_080.0 {
        return json_response(400, &serde_json::json!({"error": "use a valid timer range up to 7 days"}));
    }

    settings.enabled = enabled;
    settings.last_error = None;
    settings.next_print_at = if enabled {
        Some(common::now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()))
    } else {
        None
    };

    if let Err(error) = state::save(&config.state_path, &settings) {
        return json_response(500, &serde_json::json!({"error": format!("failed to save settings: {error}")}));
    }
    json_response(200, &serde_json::json!({"success": true, "autoprint": settings}))
}

/// Runs forever. Ticks once a second; each tick checks whether autoprint
/// is enabled and whether `next_print_at` has passed, and if so selects
/// and prints one item before scheduling the next delay. Reads state
/// fresh from disk every tick, so a restart mid-wait resumes correctly.
fn scheduler_loop(config: Arc<Config>) {
    // No lock is needed around the device write itself: printer_device::PRINT_BUSY
    // (a static AtomicBool) already serializes the one device write this process
    // ever makes, and this loop is the only caller (there is no separate "print
    // now" path in this design). We do, however, take `config.state_lock` around
    // each tick's state::load -> mutate -> state::save sequence, because that
    // sequence is *not* otherwise safe: the HTTP thread's configure_autoprint
    // handler does its own unsynchronized read-modify-write of the same state
    // file, and without this lock a POST /autoprint landing while this thread is
    // mid-print (up to 15s) would be silently overwritten by this tick's stale
    // save at the end of the loop.
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let _guard = config.state_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut settings = state::load(&config.state_path);
        // `state::load` intentionally tolerates any parseable JSON, including a
        // hand-edited state.json -- clamp to the same bounds POST /autoprint
        // enforces (min > 0, max within 7 days) so a corrupted or hand-edited
        // `min_minutes: 0, max_minutes: 0` can't make this loop fire on every
        // 1-second tick.
        settings.min_minutes = settings.min_minutes.max(1.0).min(10_080.0);
        settings.max_minutes = settings.max_minutes.max(settings.min_minutes).min(10_080.0);
        if !settings.enabled {
            continue;
        }
        let Some(next_print_at) = settings.next_print_at else {
            settings.next_print_at = Some(
                common::now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
            );
            let _ = state::save(&config.state_path, &settings);
            continue;
        };
        if common::now_unix_seconds() < next_print_at {
            continue;
        }

        let items = library::list(&config.complete_dir);
        let Some(chosen) =
            scheduler::choose_item(&items, settings.ordering, &settings.last_item_id, &mut settings.shuffle_queue, &mut rand::thread_rng()).cloned()
        else {
            settings.next_print_at = Some(
                common::now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
            );
            let _ = state::save(&config.state_path, &settings);
            continue;
        };

        let zpl_text = match library::read_zpl(&config.complete_dir, &chosen.id) {
            Ok(text) => text,
            Err(error) => {
                settings.last_error = Some(format!("failed to read print job for {}: {error}", chosen.id));
                settings.next_print_at = Some(
                    common::now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
                );
                let _ = state::save(&config.state_path, &settings);
                continue;
            }
        };

        let result = printer_device::print_zpl(&config.device_path, &zpl_text, Duration::from_secs(15));
        if result.success {
            let printed_at = common::now_unix_seconds();
            let _ = library::mark_printed(&config.complete_dir, &chosen.id, printed_at);
            settings.last_item_id = Some(chosen.id.clone());
            settings.last_error = None;
        } else {
            eprintln!("scheduler: print failed for {}: {}", chosen.id, result.message);
            settings.last_error = Some(result.message);
        }
        settings.next_print_at = Some(
            common::now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
        );
        let _ = state::save(&config.state_path, &settings);
    }
}
