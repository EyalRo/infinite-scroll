mod backend;
mod gatt;
mod handler;
mod protocol;
mod wifi;

use std::env;
use std::sync::Arc;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let backend = backend::HttpBackend {
        printer_url: env::var("PRINTER_URL").unwrap_or_else(|_| "http://127.0.0.1:8082".to_string()),
        printer_token: env::var("PRINTER_TOKEN").expect("PRINTER_TOKEN must be set"),
        uploader_url: env::var("UPLOADER_URL").unwrap_or_else(|_| "http://127.0.0.1:8081".to_string()),
        uploader_token: env::var("UPLOADER_TOKEN").expect("UPLOADER_TOKEN must be set"),
    };
    let local_name = env::var("BLE_LOCAL_NAME").unwrap_or_else(|_| "Infinite Scroll".to_string());

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("failed to start tokio runtime");
    let handler = Arc::new(handler::Handler::new(backend));
    if let Err(error) = runtime.block_on(gatt::run(handler, local_name)) {
        log::error!("bluetooth service failed: {error}");
        std::process::exit(1);
    }
}
