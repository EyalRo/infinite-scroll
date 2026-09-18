use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use common::http::{header_value, json_response};
use tiny_http::{Method, Server};

struct Config {
    token: String,
    partial_dir: PathBuf,
    ready_dir: PathBuf,
    bind_addr: String,
}

fn config_from_env() -> Config {
    Config {
        token: env::var("UPLOADER_TOKEN").expect("UPLOADER_TOKEN must be set"),
        partial_dir: PathBuf::from(env::var("UPLOADER_PARTIAL_DIR").expect("UPLOADER_PARTIAL_DIR must be set")),
        ready_dir: PathBuf::from(env::var("UPLOADER_READY_DIR").expect("UPLOADER_READY_DIR must be set")),
        bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8081".to_string()),
    }
}

fn main() {
    let config = config_from_env();
    fs::create_dir_all(&config.partial_dir).expect("failed to create partial dir");
    fs::create_dir_all(&config.ready_dir).expect("failed to create ready dir");

    let server = Server::http(&config.bind_addr).expect("failed to bind uploader HTTP server");
    eprintln!("uploader listening on {}", config.bind_addr);

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
    let path = raw_url.split('?').next().unwrap_or(&raw_url);

    if path == "/health" && *request.method() == Method::Get {
        return common::http::with_cors(json_response(200, &serde_json::json!({"status": "ok"})));
    }

    if path != "/uploads" || *request.method() != Method::Post {
        return common::http::with_cors(json_response(404, &serde_json::json!({"error": "not found"})));
    }

    let bearer = header_value(request, "Authorization");
    let access_jwt = header_value(request, "Cf-Access-Jwt-Assertion");
    if !common::auth::is_authorized(bearer, access_jwt, &config.token) {
        return common::http::with_cors(json_response(401, &serde_json::json!({"error": "unauthorized"})));
    }

    let response = if let Some(length) = request.body_length() {
        if length as u64 > common::MAX_UPLOAD_BYTES as u64 {
            json_response(413, &serde_json::json!({"error": "upload exceeds the 20 MB limit"}))
        } else {
            handle_upload(config, request)
        }
    } else {
        handle_upload(config, request)
    };

    common::http::with_cors(response)
}

fn handle_upload(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut bytes = Vec::new();
    if let Err(error) = request.as_reader().take(common::MAX_UPLOAD_BYTES as u64 + 1).read_to_end(&mut bytes) {
        return json_response(400, &serde_json::json!({"error": format!("failed to read request body: {error}")}));
    }
    if bytes.len() as u64 > common::MAX_UPLOAD_BYTES as u64 {
        return json_response(413, &serde_json::json!({"error": "upload exceeds the 20 MB limit"}));
    }

    let extension = match common::sniff_image_format(&bytes) {
        Some(common::ImageFormatKind::Png) => "png",
        Some(common::ImageFormatKind::Jpeg) => "jpg",
        None => return json_response(400, &serde_json::json!({"error": "upload is not a PNG or JPEG image"})),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{id}.{extension}");
    let partial_path = config.partial_dir.join(&filename);
    let ready_path = config.ready_dir.join(&filename);

    if let Err(error) = fs::write(&partial_path, &bytes) {
        return json_response(500, &serde_json::json!({"error": format!("failed to stage upload: {error}")}));
    }
    // Atomic on the same filesystem -- the watcher only ever sees a
    // fully-written file appear in ready_dir, never a partial one.
    if let Err(error) = fs::rename(&partial_path, &ready_path) {
        let _ = fs::remove_file(&partial_path);
        return json_response(500, &serde_json::json!({"error": format!("failed to finalize upload: {error}")}));
    }

    json_response(201, &serde_json::json!({"id": id, "filename": filename}))
}
