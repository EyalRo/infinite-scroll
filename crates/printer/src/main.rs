mod library;
mod printer_device;
mod scheduler;
mod state;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::http::{header_value, json_response};
use tiny_http::{Method, Server};

const INDEX_HTML: &str = include_str!("../../../web/library/index.html");
const STYLE_CSS: &str = include_str!("../../../web/library/style.css");
const APP_JS: &str = include_str!("../../../web/library/app.js");

struct Config {
    token: String,
    state_path: PathBuf,
    complete_dir: PathBuf,
    device_path: PathBuf,
    bind_addr: String,
    state_lock: std::sync::Mutex<()>,
}

fn config_from_env() -> Config {
    Config {
        token: env::var("PRINTER_TOKEN").expect("PRINTER_TOKEN must be set"),
        state_path: PathBuf::from(env::var("PRINTER_STATE_PATH").expect("PRINTER_STATE_PATH must be set")),
        complete_dir: PathBuf::from(env::var("PRINTER_COMPLETE_DIR").expect("PRINTER_COMPLETE_DIR must be set")),
        device_path: PathBuf::from(env::var("PRINTER_DEVICE_PATH").unwrap_or_else(|_| "/dev/usb/lp0".to_string())),
        bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8082".to_string()),
        state_lock: std::sync::Mutex::new(()),
    }
}

fn now_unix_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
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
    let url = request.url().to_string();
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
        ("/autoprint", Method::Post) => configure_autoprint(config, request),
        _ => json_response(404, &serde_json::json!({"error": "not found"})),
    };
    common::http::with_cors(response)
}

fn static_response(body: &'static str, content_type: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
        .expect("static header name/value is always valid");
    tiny_http::Response::from_data(body.as_bytes().to_vec()).with_header(header)
}

fn status_response(config: &Config) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let settings = state::load(&config.state_path);
    let items = library::list(&config.complete_dir);
    json_response(
        200,
        &serde_json::json!({
            "printer": {
                "connected": printer_device::is_available(&config.device_path),
                "busy": printer_device::is_busy(),
            },
            "catalog_count": items.len(),
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

    let _guard = config.state_lock.lock().unwrap();
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
        Some(now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()))
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
        let _guard = config.state_lock.lock().unwrap();
        let mut settings = state::load(&config.state_path);
        if !settings.enabled {
            continue;
        }
        let Some(next_print_at) = settings.next_print_at else {
            settings.next_print_at = Some(
                now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
            );
            let _ = state::save(&config.state_path, &settings);
            continue;
        };
        if now_unix_seconds() < next_print_at {
            continue;
        }

        let items = library::list(&config.complete_dir);
        let Some(chosen) = scheduler::choose_item(&items, settings.ordering, &settings.last_item_id, &mut rand::thread_rng()).cloned() else {
            settings.next_print_at = Some(
                now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
            );
            let _ = state::save(&config.state_path, &settings);
            continue;
        };

        let zpl_text = match library::read_zpl(&config.complete_dir, &chosen.id) {
            Ok(text) => text,
            Err(error) => {
                settings.last_error = Some(format!("failed to read print job for {}: {error}", chosen.id));
                settings.next_print_at = Some(
                    now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
                );
                let _ = state::save(&config.state_path, &settings);
                continue;
            }
        };

        let result = printer_device::print_zpl(&config.device_path, &zpl_text, Duration::from_secs(15));
        if result.success {
            let printed_at = now_unix_seconds();
            let _ = library::mark_printed(&config.complete_dir, &chosen.id, printed_at);
            settings.last_item_id = Some(chosen.id.clone());
            settings.last_error = None;
        } else {
            eprintln!("scheduler: print failed for {}: {}", chosen.id, result.message);
            settings.last_error = Some(result.message);
        }
        settings.next_print_at = Some(
            now_unix_seconds() + scheduler::random_delay_seconds(settings.min_minutes, settings.max_minutes, &mut rand::thread_rng()),
        );
        let _ = state::save(&config.state_path, &settings);
    }
}
