# Rust Print Services Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the entire Python/Flask `infinite-scroll` app with three small, independent Rust services (uploader, watcher/converter, printer) plus a static mobile-first web frontend, deployed to the Raspberry Pi 4, each reachable at its own `*.infinite-scroll.art.virtualdino.com` hostname behind Cloudflare Access.

**Architecture:** `uploader` accepts PNG/JPG over a single PUT-style endpoint and stages files through `partial/` → `ready/` via atomic rename. `watcher` is a pure filesystem loop (no network, no API) that converts `ready/` files to the validated 650-dot/Floyd-Steinberg/ZPL print format and moves them into `complete/` (the library) with a JSON metadata sidecar. `printer` owns the library (list/remove), the autoprint scheduler, and the actual bounded/serialized write to `/dev/usb/lp0`; it is the only one of the three built for indefinite unattended offline operation. A static HTML/CSS/JS frontend (no backend of its own) calls `uploader` and `printer` directly from the browser, authenticated the same way MediaWatch is (Cloudflare Access human session + service token on one Access Application per hostname).

**Tech Stack:** Rust (workspace of 3 binaries + 1 shared lib crate), `tiny_http` (sync, minimal-dependency HTTP server — no tokio), `image` crate (`jpeg`+`png` features only, `default-features = false` — same choice already validated in `plugins/virtualdino` for `wasm32-unknown-unknown`; here it targets `aarch64-unknown-linux-gnu`), `serde`/`serde_json`, `uuid`. No SQLite, no C dependencies beyond what `image`'s codecs need (none — both codecs are pure Rust). Cross-compiled from the NixOS dev host to `aarch64-unknown-linux-gnu`, deployed over SSH to `infinite-scroll.local` (confirmed reachable, Debian trixie, aarch64, 4 cores, 3.7GB RAM, gcc present, no `cloudflared` on the device itself — the Tunnel connector runs on `pve2`, which this session cannot reach).

## Global Constraints

- Images only: PNG or JPEG. No structured/LinkedIn-style post type anywhere in the new system.
- No EXIF auto-rotation in v1 — a deliberate simplification versus the old Python pipeline (which used `ImageOps.exif_transpose`); note this as a known follow-up, not a silent gap.
- The validated print contract must be preserved exactly: 650-dot canvas width, grayscale, Floyd-Steinberg 1-bit dithering, `^GFA` ZPL packing (8 dots/byte, black = 1 bit, uppercase hex), written to `/dev/usb/lp0`.
- `printer`'s device write must stay serialized (one job at a time) and bounded (15s timeout) — this is the exact fix for a real 2026-09-13 incident (a stuck write hanging the whole process).
- Every service binds to a loopback/private address only; the only ingress path is the Cloudflare Tunnel. No service independently verifies the `Cf-Access-Jwt-Assertion` JWT (no JWKS/crypto dependency) — its mere presence is treated as evidence the request passed through Access, since nothing else can reach these ports. A separate shared-secret `Authorization: Bearer` header is required for service/MCP callers as defense in depth, matching the existing `INFINITE_SCROLL_MCP_TOKEN` pattern already used in production.
- Cross-compile to `aarch64-unknown-linux-gnu`; do not build on the Pi.
- No dependency requiring a C toolchain beyond `gcc` for final linking (already present on the Pi and available via `nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc` on the dev host).
- End state: old Python app (`app/`, `tests/`, `run.py`, `requirements*.txt`, `pyproject.toml`, `deploy/infinite-scroll-webapp.service`) fully removed from the repo and fully decommissioned on the Pi (service stopped, disabled, files deleted). Everything lands on `main` (this repo's current branch) with no other branches left behind.
- Cloudflare Tunnel ingress rules, Access Applications, and DNS for the three new hostnames are **out of scope for autonomous execution** — this session has no SSH path to `pve2` and no Cloudflare API scope beyond `zone:read`. Task 11 documents the exact manual steps instead.

---

## File Structure

```
infinite-scroll/
  Cargo.toml                          # workspace
  crates/
    common/
      Cargo.toml
      src/lib.rs                      # image normalize, dither, ZPL pack, auth check
      src/dither.rs
      src/zpl.rs
      src/auth.rs
    uploader/
      Cargo.toml
      src/main.rs
    watcher/
      Cargo.toml
      src/main.rs
    printer/
      Cargo.toml
      src/main.rs
      src/state.rs                    # JSON-file-backed scheduler settings
      src/library.rs                  # catalog list/add-metadata/remove
      src/printer_device.rs           # serialized, bounded /dev/usb/lp0 write
      src/scheduler.rs                # background tick loop
  web/
    library/
      index.html
      app.js
      style.css
  deploy/
    uploader.service
    watcher.service
    printer.service
    deploy.sh                         # cross-compile + scp + restart
  docs/
    superpowers/plans/2026-09-17-rust-print-services.md   # this file
```

Files deleted in Task 10: `app/`, `tests/`, `run.py`, `requirements.txt`, `requirements-dev.txt`, `pyproject.toml`, `deploy/infinite-scroll-webapp.service`, `.pytest_cache/`, `__pycache__/`.

---

### Task 1: Workspace scaffold and the `common` crate — image normalize, dither, ZPL pack

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/common/Cargo.toml`
- Create: `crates/common/src/lib.rs`
- Create: `crates/common/src/dither.rs`
- Create: `crates/common/src/zpl.rs`
- Create: `crates/common/src/auth.rs`

**Interfaces:**
- Produces: `common::normalize_and_convert(bytes: &[u8]) -> Result<ZplJob, ConvertError>` — the single entry point Task 4 (watcher) calls. `ZplJob { pub text: String, pub width: u32, pub height: u32 }`. `ConvertError` is a `thiserror`-free plain enum with a `Display` impl carrying a human-readable message.
- Produces: `common::sniff_image_format(bytes: &[u8]) -> Option<ImageFormatKind>` where `ImageFormatKind { Png, Jpeg }` — Task 3 (uploader) uses this to validate uploads before staging them.
- Produces: `common::auth::is_authorized(bearer_header: Option<&str>, access_jwt_header: Option<&str>, expected_token: &str) -> bool` — Tasks 3 and 5 (uploader, printer) use this for every request.

- [ ] **Step 1: Create the workspace root**

```toml
# Cargo.toml
[workspace]
members = ["crates/common", "crates/uploader", "crates/watcher", "crates/printer"]
resolver = "2"

[profile.release]
opt-level = "z"
lto = true
strip = true
codegen-units = 1
```

- [ ] **Step 2: Create `crates/common/Cargo.toml`**

```toml
[package]
name = "common"
version = "0.1.0"
edition = "2021"

[dependencies]
image = { version = "0.25", default-features = false, features = ["jpeg", "png"] }
```

- [ ] **Step 3: Write `crates/common/src/zpl.rs` (ported from `app/dither.py`'s `pack_to_zpl`, verified byte-identical algorithm)**

```rust
/// A complete, self-contained ZPL job ready to write to the printer device.
pub struct ZplJob {
    pub text: String,
    pub width: u32,
    pub height: u32,
}

/// Packs a 1-bit image (one byte per pixel, 0 = black, non-zero = white --
/// the shape `dither::to_1bit` produces) into a self-contained ZPL `^GFA`
/// job. Matches the validated contract exactly: 8 dots per byte, MSB
/// first, black = 1 bit, short rows padded with trailing zero bits,
/// uppercase hex.
pub fn pack_to_zpl(bits: &[u8], width: u32, height: u32) -> ZplJob {
    let row_bytes = (width as usize + 7) / 8;
    let total_bytes = row_bytes * height as usize;
    let mut out = vec![0u8; total_bytes];

    for y in 0..height as usize {
        let mut byte = 0u8;
        let mut bitcount = 0u8;
        let mut out_idx = y * row_bytes;
        for x in 0..width as usize {
            let black = if bits[y * width as usize + x] == 0 { 1u8 } else { 0u8 };
            byte = (byte << 1) | black;
            bitcount += 1;
            if bitcount == 8 {
                out[out_idx] = byte;
                out_idx += 1;
                byte = 0;
                bitcount = 0;
            }
        }
        if bitcount > 0 {
            byte <<= 8 - bitcount;
            out[out_idx] = byte;
        }
    }

    let hexdata = out.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let text = format!(
        "^XA\n^PW{width}\n^LL{height}\n^LH0,0\n^FO0,0\n^GFA,{total_bytes},{total_bytes},{row_bytes},{hexdata}\n^XZ\n"
    );
    ZplJob { text, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_a_2x2_checkerboard_with_msb_first_and_trailing_zero_padding() {
        // black, white
        // white, black  -- as a flat row-major 1-bit-per-pixel buffer (0 = black)
        let bits = [0u8, 255, 255, 0];
        let job = pack_to_zpl(&bits, 2, 2);
        // row_bytes = ceil(2/8) = 1, total_bytes = 2
        // row 0: black(1) white(0) -> bits "10" -> padded "10000000" -> 0x80
        // row 1: white(0) black(1) -> bits "01" -> padded "01000000" -> 0x40
        assert!(job.text.contains("^GFA,2,2,1,8040\n"));
        assert!(job.text.starts_with("^XA\n^PW2\n^LL2\n"));
        assert!(job.text.ends_with("^XZ\n"));
    }

    #[test]
    fn packs_a_full_byte_row_with_no_padding() {
        let bits = [0u8, 0, 0, 0, 0, 0, 0, 0]; // 8 black pixels, 1 row
        let job = pack_to_zpl(&bits, 8, 1);
        assert!(job.text.contains("^GFA,1,1,1,FF\n"));
    }
}
```

- [ ] **Step 4: Write `crates/common/src/dither.rs` (Floyd-Steinberg error diffusion on a grayscale buffer)**

```rust
/// Converts a grayscale (one byte per pixel, 0-255) buffer to 1-bit using
/// Floyd-Steinberg error diffusion -- the same algorithm PIL's `L -> 1`
/// conversion uses (its documented default), which is what the validated
/// print contract was built and physically tested against.
///
/// Output: one byte per pixel, `0` = black, `255` = white -- same shape
/// `zpl::pack_to_zpl` consumes, kept byte-per-pixel (not bit-packed) so the
/// error-diffusion arithmetic stays simple and bounds-obviously-correct.
pub fn floyd_steinberg_1bit(gray: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut errors: Vec<f32> = gray.iter().map(|&p| p as f32).collect();
    let mut out = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let old = errors[idx].clamp(0.0, 255.0);
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            out[idx] = new as u8;
            let err = old - new;

            let mut push = |dx: isize, dy: isize, weight: f32| {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                if nx >= 0 && (nx as usize) < width && ny >= 0 && (ny as usize) < height {
                    errors[ny as usize * width + nx as usize] += err * weight;
                }
            };
            push(1, 0, 7.0 / 16.0);
            push(-1, 1, 3.0 / 16.0);
            push(0, 1, 5.0 / 16.0);
            push(1, 1, 1.0 / 16.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_black_image_stays_black() {
        let gray = vec![0u8; 16];
        let out = floyd_steinberg_1bit(&gray, 4, 4);
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn a_uniform_white_image_stays_white() {
        let gray = vec![255u8; 16];
        let out = floyd_steinberg_1bit(&gray, 4, 4);
        assert!(out.iter().all(|&p| p == 255));
    }

    #[test]
    fn a_mid_gray_image_produces_a_mix_of_black_and_white() {
        let gray = vec![128u8; 400];
        let out = floyd_steinberg_1bit(&gray, 20, 20);
        let black = out.iter().filter(|&&p| p == 0).count();
        let white = out.iter().filter(|&&p| p == 255).count();
        assert!(black > 50 && white > 50, "expected a dithered mix, got {black} black / {white} white");
    }
}
```

- [ ] **Step 5: Write `crates/common/src/auth.rs`**

```rust
/// A request is authorized if either:
///  - it carries the shared-secret bearer token (a service/MCP caller), or
///  - it carries a non-empty Cf-Access-Jwt-Assertion header (a browser
///    request that already passed Cloudflare Access at the edge -- these
///    services are only reachable through the Access-protected Tunnel, so
///    presence alone is sufficient; no local JWKS/JWT verification is done).
pub fn is_authorized(bearer_header: Option<&str>, access_jwt_header: Option<&str>, expected_token: &str) -> bool {
    if let Some(value) = bearer_header {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
                return true;
            }
        }
    }
    matches!(access_jwt_header, Some(value) if !value.trim().is_empty())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_correct_bearer_token() {
        assert!(is_authorized(Some("Bearer secret"), None, "secret"));
    }

    #[test]
    fn rejects_a_wrong_bearer_token() {
        assert!(!is_authorized(Some("Bearer wrong"), None, "secret"));
    }

    #[test]
    fn accepts_a_present_access_jwt_header_with_no_bearer_token() {
        assert!(is_authorized(None, Some("some.jwt.value"), "secret"));
    }

    #[test]
    fn rejects_an_empty_access_jwt_header_and_no_bearer_token() {
        assert!(!is_authorized(None, Some("   "), "secret"));
    }

    #[test]
    fn rejects_when_neither_header_is_present() {
        assert!(!is_authorized(None, None, "secret"));
    }
}
```

- [ ] **Step 6: Write `crates/common/src/lib.rs` (ties normalize + dither + zpl together, ported from `app/image_pipeline.py`'s `normalize_uploaded_image`)**

```rust
pub mod auth;
pub mod dither;
pub mod zpl;

pub use zpl::ZplJob;

/// The validated print canvas width (dots). See the Infinite Scroll
/// knowledge page's "Validated printing method" section.
pub const PRINT_WIDTH_PX: u32 = 650;
const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 50_000_000;
const MAX_HEIGHT_PX: u32 = 20_000;

#[derive(Debug)]
pub enum ImageFormatKind {
    Png,
    Jpeg,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_SIGNATURE: [u8; 3] = [0xff, 0xd8, 0xff];

/// Sniffs the magic bytes only -- deliberately not relying on a
/// caller-supplied Content-Type or filename extension, both of which are
/// trivially wrong or spoofable.
pub fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormatKind> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        Some(ImageFormatKind::Png)
    } else if bytes.starts_with(&JPEG_SIGNATURE) {
        Some(ImageFormatKind::Jpeg)
    } else {
        None
    }
}

#[derive(Debug)]
pub struct ConvertError(pub String);

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decodes, flattens transparency onto white, converts to grayscale,
/// resizes to PRINT_WIDTH_PX preserving aspect ratio, dithers to 1-bit,
/// and packs into a ZPL job. The single entry point the watcher calls for
/// every file it picks up from `ready/`.
pub fn normalize_and_convert(bytes: &[u8]) -> Result<ZplJob, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError("upload is empty".into()));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ConvertError("image is larger than 20 MB".into()));
    }
    if sniff_image_format(bytes).is_none() {
        return Err(ConvertError("not a PNG or JPEG image".into()));
    }

    let decoded = image::load_from_memory(bytes)
        .map_err(|error| ConvertError(format!("failed to decode image: {error}")))?;

    if (decoded.width() as u64) * (decoded.height() as u64) > MAX_IMAGE_PIXELS {
        return Err(ConvertError("image dimensions are too large".into()));
    }

    // Flatten any alpha onto white, then convert to grayscale -- matches
    // the Python pipeline's RGBA-composite-then-L-convert behavior for
    // images with transparency, and is a no-op (transparency == fully
    // opaque) for opaque images either way.
    let rgba = decoded.to_rgba8();
    let mut flattened = image::ImageBuffer::from_pixel(rgba.width(), rgba.height(), image::Rgb([255u8, 255, 255]));
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let alpha = a as f32 / 255.0;
        let blend = |channel: u8, background: u8| -> u8 {
            (channel as f32 * alpha + background as f32 * (1.0 - alpha)).round() as u8
        };
        flattened.put_pixel(x, y, image::Rgb([blend(r, 255), blend(g, 255), blend(b, 255)]));
    }
    let gray = image::DynamicImage::ImageRgb8(flattened).to_luma8();

    let height = ((gray.height() as f64) * (PRINT_WIDTH_PX as f64) / (gray.width() as f64))
        .round()
        .max(1.0) as u32;
    if height > MAX_HEIGHT_PX {
        return Err(ConvertError(format!("image is too tall after scaling (maximum {MAX_HEIGHT_PX} dots)")));
    }
    let resized = image::imageops::resize(&gray, PRINT_WIDTH_PX, height, image::imageops::FilterType::Lanczos3);

    let bits = dither::floyd_steinberg_1bit(resized.as_raw(), PRINT_WIDTH_PX, height);
    Ok(zpl::pack_to_zpl(&bits, PRINT_WIDTH_PX, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 10, 10]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn rejects_empty_input() {
        let error = normalize_and_convert(&[]).unwrap_err();
        assert_eq!(error.0, "upload is empty");
    }

    #[test]
    fn rejects_bytes_that_are_not_png_or_jpeg() {
        let error = normalize_and_convert(b"not an image").unwrap_err();
        assert_eq!(error.0, "not a PNG or JPEG image");
    }

    #[test]
    fn converts_a_tiny_png_to_the_validated_print_width() {
        let job = normalize_and_convert(&tiny_png_bytes(100, 50)).unwrap();
        assert_eq!(job.width, PRINT_WIDTH_PX);
        assert_eq!(job.height, 325); // 50 * 650 / 100
        assert!(job.text.starts_with("^XA\n^PW650\n"));
    }

    #[test]
    fn sniffs_png_and_jpeg_by_magic_bytes_not_by_extension() {
        assert!(matches!(sniff_image_format(&tiny_png_bytes(1, 1)), Some(ImageFormatKind::Png)));
        assert!(sniff_image_format(b"garbage").is_none());
    }
}
```

- [ ] **Step 7: Run the common crate's tests**

Run: `cd crates/common && cargo test`
Expected: all tests pass (9 tests: 2 zpl, 3 dither, 5 auth, 4 lib — 14 total).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/common
git commit -m "Add Rust workspace and common crate: image normalize, dither, ZPL pack, auth"
```

---

### Task 2: `tiny_http`-based HTTP helper shared shape (request parsing conventions)

This task exists only to lock in the exact `tiny_http` request-handling pattern both `uploader` and `printer` will repeat, so Tasks 3 and 5 aren't inventing it twice inconsistently. No new crate — `common` gains one more module.

**Files:**
- Create: `crates/common/src/http.rs`
- Modify: `crates/common/src/lib.rs:1-3` (add `pub mod http;`)

**Interfaces:**
- Produces: `common::http::json_response(status: u16, body: &serde_json::Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>>`
- Produces: `common::http::header_value<'a>(request: &'a tiny_http::Request, name: &str) -> Option<&'a str>`
- Consumes: `tiny_http` types (added as a dependency here).

- [ ] **Step 1: Add `tiny_http` to `crates/common/Cargo.toml`**

```toml
[dependencies]
image = { version = "0.25", default-features = false, features = ["jpeg", "png"] }
tiny_http = "0.12"
serde_json = "1"
```

- [ ] **Step 2: Write `crates/common/src/http.rs`**

```rust
use tiny_http::{Header, Request, Response};

/// Builds a JSON response with the given status code. `tiny_http`'s
/// `Response::from_data` takes ownership of the body bytes, so this
/// returns the concrete boxed-cursor type directly rather than trying to
/// name an unnameable closure type.
pub fn json_response(status: u16, body: &serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header name/value is always valid");
    Response::from_data(bytes).with_status_code(status).with_header(header)
}

/// Case-insensitive header lookup -- HTTP header names are case-insensitive
/// and `tiny_http` does not normalize them for you.
pub fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_response_sets_the_content_type_and_status() {
        let response = json_response(201, &serde_json::json!({"ok": true}));
        assert_eq!(response.status_code().0, 201);
    }
}
```

- [ ] **Step 3: Add the `pub mod http;` line**

```rust
// crates/common/src/lib.rs, at the top alongside the existing module declarations
pub mod auth;
pub mod dither;
pub mod http;
pub mod zpl;
```

- [ ] **Step 4: Run tests**

Run: `cd crates/common && cargo test`
Expected: all previous tests plus the new `json_response_sets_the_content_type_and_status` test pass.

- [ ] **Step 5: Commit**

```bash
git add crates/common
git commit -m "Add shared tiny_http response/header helpers to common"
```

---

### Task 3: `uploader` binary

**Files:**
- Create: `crates/uploader/Cargo.toml`
- Create: `crates/uploader/src/main.rs`

**Interfaces:**
- Consumes: `common::sniff_image_format`, `common::auth::is_authorized`, `common::http::{json_response, header_value}` (Tasks 1-2).
- Environment variables (all required except `BIND_ADDR`): `UPLOADER_TOKEN` (shared secret), `UPLOADER_PARTIAL_DIR`, `UPLOADER_READY_DIR`, `BIND_ADDR` (default `127.0.0.1:8081`).
- Produces: files named `<uuid>.png` or `<uuid>.jpg` appearing in `UPLOADER_READY_DIR` once an upload completes -- Task 4 (watcher) consumes these by polling that directory.

- [ ] **Step 1: Write `crates/uploader/Cargo.toml`**

```toml
[package]
name = "uploader"
version = "0.1.0"
edition = "2021"

[dependencies]
common = { path = "../common" }
tiny_http = "0.12"
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: Write `crates/uploader/src/main.rs`**

```rust
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use common::http::{header_value, json_response};
use tiny_http::{Method, Server};

const MAX_BODY_BYTES: u64 = 20 * 1024 * 1024;

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
    if request.url() == "/health" && *request.method() == Method::Get {
        return json_response(200, &serde_json::json!({"status": "ok"}));
    }

    if request.url() != "/uploads" || *request.method() != Method::Post {
        return json_response(404, &serde_json::json!({"error": "not found"}));
    }

    let bearer = header_value(request, "Authorization");
    let access_jwt = header_value(request, "Cf-Access-Jwt-Assertion");
    if !common::auth::is_authorized(bearer, access_jwt, &config.token) {
        return json_response(401, &serde_json::json!({"error": "unauthorized"}));
    }

    if let Some(length) = request.body_length() {
        if length as u64 > MAX_BODY_BYTES {
            return json_response(413, &serde_json::json!({"error": "upload exceeds the 20 MB limit"}));
        }
    }

    let mut bytes = Vec::new();
    if let Err(error) = request.as_reader().take(MAX_BODY_BYTES + 1).read_to_end(&mut bytes) {
        return json_response(400, &serde_json::json!({"error": format!("failed to read request body: {error}")}));
    }
    if bytes.len() as u64 > MAX_BODY_BYTES {
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
```

- [ ] **Step 3: Build to confirm it compiles**

Run: `cd crates/uploader && cargo build`
Expected: builds with no errors (this crate has no unit tests of its own -- its logic is a thin wrapper over already-tested `common` functions; behavior is exercised end-to-end in Task 9).

- [ ] **Step 4: Commit**

```bash
git add crates/uploader
git commit -m "Add uploader binary: PUT-style PNG/JPG upload with partial-then-ready staging"
```

---

### Task 4: `watcher` binary

**Files:**
- Create: `crates/watcher/Cargo.toml`
- Create: `crates/watcher/src/main.rs`

**Interfaces:**
- Consumes: `common::normalize_and_convert` (Task 1), reads files from `WATCHER_READY_DIR` (produced by Task 3's uploader).
- Produces: for each `<uuid>.<ext>` in `ready/`, writes `<uuid>.zpl` and `<uuid>.json` (metadata sidecar: `{"id": "<uuid>", "original_filename": "<uuid>.<ext>", "added_at": <unix seconds f64>, "print_count": 0, "last_printed_at": null}`) into `WATCHER_COMPLETE_DIR` -- Task 5 (printer) reads this directory and this exact sidecar shape.
- Environment variables: `WATCHER_READY_DIR`, `WATCHER_COMPLETE_DIR`, `WATCHER_FAILED_DIR`, `WATCHER_POLL_SECONDS` (default `3`).

- [ ] **Step 1: Write `crates/watcher/Cargo.toml`**

```toml
[package]
name = "watcher"
version = "0.1.0"
edition = "2021"

[dependencies]
common = { path = "../common" }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: Write `crates/watcher/src/main.rs`**

```rust
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
```

- [ ] **Step 3: Build to confirm it compiles**

Run: `cd crates/watcher && cargo build`
Expected: builds with no errors.

- [ ] **Step 4: Manual verification of the ready → complete flow**

```bash
cd crates/watcher
mkdir -p /tmp/watcher-test/{ready,complete,failed}
WATCHER_READY_DIR=/tmp/watcher-test/ready \
WATCHER_COMPLETE_DIR=/tmp/watcher-test/complete \
WATCHER_FAILED_DIR=/tmp/watcher-test/failed \
WATCHER_POLL_SECONDS=1 \
cargo run &
sleep 1
# Write a tiny real PNG into ready/ using the common crate's own test helper via a throwaway script, or copy any small PNG you have locally to /tmp/watcher-test/ready/test-id.png
sleep 3
ls /tmp/watcher-test/complete/   # expect test-id.zpl and test-id.json
kill %1
rm -rf /tmp/watcher-test
```

Expected: `test-id.zpl` and `test-id.json` appear in `complete/`; the sidecar's `id` field reads `"test-id"`.

- [ ] **Step 5: Commit**

```bash
git add crates/watcher
git commit -m "Add watcher binary: polls ready/, converts, writes to complete/ with a metadata sidecar"
```

---

### Task 5: `printer` binary — state, library, and the serialized/bounded device write

**Files:**
- Create: `crates/printer/Cargo.toml`
- Create: `crates/printer/src/state.rs`
- Create: `crates/printer/src/library.rs`
- Create: `crates/printer/src/printer_device.rs`

**Interfaces:**
- Produces: `state::Settings { enabled: bool, min_minutes: f64, max_minutes: f64, ordering: Ordering, next_print_at: Option<f64>, last_item_id: Option<String>, last_error: Option<String> }`, `state::load(path: &Path) -> Settings`, `state::save(path: &Path, settings: &Settings) -> std::io::Result<()>`.
- Produces: `library::CatalogItem { id, original_filename, added_at, print_count, last_printed_at }`, `library::list(complete_dir: &Path) -> Vec<CatalogItem>`, `library::remove(complete_dir: &Path, id: &str) -> Option<CatalogItem>`, `library::mark_printed(complete_dir: &Path, id: &str, printed_at: f64) -> std::io::Result<()>`.
- Produces: `printer_device::PrintResult { success: bool, message: String }`, `printer_device::is_available(device_path: &Path) -> bool`, `printer_device::is_busy() -> bool`, `printer_device::print_zpl(device_path: &Path, zpl_text: &str, timeout: Duration) -> PrintResult`.

- [ ] **Step 1: Write `crates/printer/Cargo.toml`**

```toml
[package]
name = "printer"
version = "0.1.0"
edition = "2021"

[dependencies]
common = { path = "../common" }
tiny_http = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rand = "0.8"
```

- [ ] **Step 2: Write `crates/printer/src/state.rs` (ported from `backlog.py`'s `scheduler_settings` table; JSON file instead of SQLite -- no `run_mode`/`once` since the LinkedIn/once-through backlog concept is dropped, this installation always loops)**

```rust
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Ordering {
    Random,
    Sequential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub min_minutes: f64,
    pub max_minutes: f64,
    pub ordering: Ordering,
    pub next_print_at: Option<f64>,
    pub last_item_id: Option<String>,
    pub last_error: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            min_minutes: 15.0,
            max_minutes: 20.0,
            ordering: Ordering::Sequential,
            next_print_at: None,
            last_item_id: None,
            last_error: None,
        }
    }
}

/// Missing or unreadable state file -> safe defaults (disabled). Corrupt
/// JSON is treated the same way rather than crashing the whole service on
/// startup -- an unattended, offline installation must never fail to boot
/// because of a damaged state file.
pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes to a temp file in the same directory, then renames -- atomic on
/// the same filesystem, so a crash mid-write can never leave a truncated
/// state file that `load` would silently treat as "reset to defaults".
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, serde_json::to_vec_pretty(settings).unwrap())?;
    fs::rename(&temp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_the_file_does_not_exist() {
        let settings = load(Path::new("/tmp/does-not-exist-infinite-scroll-state.json"));
        assert!(!settings.enabled);
        assert_eq!(settings.ordering, Ordering::Sequential);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = std::env::temp_dir().join(format!("printer-state-test-{}.json", std::process::id()));
        let mut settings = Settings::default();
        settings.enabled = true;
        settings.ordering = Ordering::Random;
        settings.last_item_id = Some("abc".into());
        save(&path, &settings).unwrap();
        let loaded = load(&path);
        assert!(loaded.enabled);
        assert_eq!(loaded.ordering, Ordering::Random);
        assert_eq!(loaded.last_item_id, Some("abc".into()));
        let _ = fs::remove_file(&path);
    }
}
```

- [ ] **Step 3: Write `crates/printer/src/library.rs` (ported from `backlog.py`'s `list_items`/`delete_item`/`mark_printed`, reading the sidecar files Task 4's watcher writes)**

```rust
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub original_filename: String,
    pub added_at: f64,
    pub print_count: u32,
    pub last_printed_at: Option<f64>,
}

fn sidecar_path(complete_dir: &Path, id: &str) -> std::path::PathBuf {
    complete_dir.join(format!("{id}.json"))
}

fn zpl_path(complete_dir: &Path, id: &str) -> std::path::PathBuf {
    complete_dir.join(format!("{id}.zpl"))
}

/// Lists every catalog item, oldest-added first -- a stable, predictable
/// order for "sequential" scheduling and for the library-management UI.
pub fn list(complete_dir: &Path) -> Vec<CatalogItem> {
    let Ok(entries) = fs::read_dir(complete_dir) else {
        return Vec::new();
    };
    let mut items: Vec<CatalogItem> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<CatalogItem>(&text).ok())
        .collect();
    items.sort_by(|a, b| a.added_at.partial_cmp(&b.added_at).unwrap_or(std::cmp::Ordering::Equal));
    items
}

pub fn get(complete_dir: &Path, id: &str) -> Option<CatalogItem> {
    let text = fs::read_to_string(sidecar_path(complete_dir, id)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Removes both the sidecar and the print-ready ZPL file. Returns the
/// removed item's metadata so the caller (the HTTP API) can echo it back,
/// or `None` if no such item exists (the caller answers 404).
pub fn remove(complete_dir: &Path, id: &str) -> Option<CatalogItem> {
    let item = get(complete_dir, id)?;
    let _ = fs::remove_file(zpl_path(complete_dir, id));
    let _ = fs::remove_file(sidecar_path(complete_dir, id));
    Some(item)
}

pub fn read_zpl(complete_dir: &Path, id: &str) -> std::io::Result<String> {
    fs::read_to_string(zpl_path(complete_dir, id))
}

pub fn mark_printed(complete_dir: &Path, id: &str, printed_at: f64) -> std::io::Result<()> {
    let Some(mut item) = get(complete_dir, id) else {
        return Ok(()); // Item was removed between selection and printing -- nothing to update.
    };
    item.print_count += 1;
    item.last_printed_at = Some(printed_at);
    fs::write(sidecar_path(complete_dir, id), serde_json::to_vec_pretty(&item).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_item(dir: &Path, id: &str, added_at: f64) {
        let item = CatalogItem {
            id: id.to_string(),
            original_filename: format!("{id}.png"),
            added_at,
            print_count: 0,
            last_printed_at: None,
        };
        fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&item).unwrap()).unwrap();
        fs::write(dir.join(format!("{id}.zpl")), b"^XA\n^XZ\n").unwrap();
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("library-test-{}-{}", std::process::id(), rand_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn list_returns_items_oldest_first() {
        let dir = temp_dir();
        write_item(&dir, "second", 200.0);
        write_item(&dir, "first", 100.0);
        let items = list(&dir);
        assert_eq!(items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), vec!["first", "second"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_deletes_both_files_and_returns_the_item() {
        let dir = temp_dir();
        write_item(&dir, "one", 1.0);
        let removed = remove(&dir, "one").unwrap();
        assert_eq!(removed.id, "one");
        assert!(!dir.join("one.json").exists());
        assert!(!dir.join("one.zpl").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_returns_none_for_an_unknown_id() {
        let dir = temp_dir();
        assert!(remove(&dir, "missing").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_printed_increments_the_count_and_sets_last_printed_at() {
        let dir = temp_dir();
        write_item(&dir, "one", 1.0);
        mark_printed(&dir, "one", 500.0).unwrap();
        let item = get(&dir, "one").unwrap();
        assert_eq!(item.print_count, 1);
        assert_eq!(item.last_printed_at, Some(500.0));
        let _ = fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 4: Write `crates/printer/src/printer_device.rs` (ported from `app/printer.py` exactly -- same preflight checks, same serialized lock, same 15s bounded write)**

```rust
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

pub struct PrintResult {
    pub success: bool,
    pub message: String,
}

static PRINT_LOCK: Mutex<()> = Mutex::new(());

pub fn is_available(device_path: &Path) -> bool {
    std::fs::metadata(device_path).map(|meta| meta.file_type().is_char_device()).unwrap_or(false)
}

pub fn is_busy() -> bool {
    PRINT_LOCK.try_lock().is_err()
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

    let Ok(_guard) = PRINT_LOCK.try_lock() else {
        return PrintResult {
            success: false,
            message: "printer busy: a previous print is still in progress".to_string(),
        };
    };

    let (sender, receiver) = mpsc::channel();
    let device_path = device_path.to_path_buf();
    let zpl_text = zpl_text.to_string();
    std::thread::spawn(move || {
        let outcome = OpenOptions::new()
            .write(true)
            .open(&device_path)
            .and_then(|mut file| file.write_all(zpl_text.as_bytes()).and_then(|_| file.flush()));
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
```

- [ ] **Step 5: Run all printer crate tests so far**

Run: `cd crates/printer && cargo test`
Expected: 2 state tests + 4 library tests + 3 printer_device tests = 9 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/printer/Cargo.toml crates/printer/src/state.rs crates/printer/src/library.rs crates/printer/src/printer_device.rs
git commit -m "Add printer crate's state, library, and printer_device modules"
```

---

### Task 6: `printer` binary — scheduler and HTTP API (`main.rs`, `scheduler.rs`)

**Files:**
- Create: `crates/printer/src/scheduler.rs`
- Create: `crates/printer/src/main.rs`

**Interfaces:**
- Consumes: everything from Task 5 (`state`, `library`, `printer_device` modules) plus `common::auth::is_authorized` and `common::http::{json_response, header_value}`.
- Produces: the HTTP API Task 8 (frontend) and the MCP layer (Task 9) both call: `GET /health`, `GET /status`, `GET /catalog`, `DELETE /catalog/{id}`, `POST /autoprint`.
- Environment variables: `PRINTER_TOKEN`, `PRINTER_STATE_PATH`, `PRINTER_COMPLETE_DIR`, `PRINTER_DEVICE_PATH` (default `/dev/usb/lp0`), `BIND_ADDR` (default `127.0.0.1:8082`).

- [ ] **Step 1: Write `crates/printer/src/scheduler.rs` (ported from `app/scheduler.py`'s `choose_item`/`BacklogScheduler`, simplified: no `run_mode` since it's dropped, always loops)**

```rust
use rand::seq::SliceRandom;
use rand::Rng;

use crate::library::CatalogItem;
use crate::state::Ordering;

/// Picks the next item to print. `random` chooses uniformly at random,
/// avoiding an immediate repeat of `last_item_id` when more than one item
/// exists. `sequential` advances past `last_item_id` in list order,
/// wrapping around, or starts at the first item if there is no last item
/// (fresh state, or the last-printed item was since removed).
pub fn choose_item<'a>(items: &'a [CatalogItem], ordering: Ordering, last_item_id: &Option<String>, rng: &mut impl Rng) -> Option<&'a CatalogItem> {
    if items.is_empty() {
        return None;
    }
    match ordering {
        Ordering::Random => {
            let candidates: Vec<&CatalogItem> = items.iter().filter(|item| Some(&item.id) != last_item_id.as_ref()).collect();
            let pool = if candidates.is_empty() { items.iter().collect::<Vec<_>>() } else { candidates };
            pool.choose(rng).copied()
        }
        Ordering::Sequential => {
            let last_index = last_item_id.as_ref().and_then(|id| items.iter().position(|item| &item.id == id));
            let next_index = last_index.map(|index| (index + 1) % items.len()).unwrap_or(0);
            items.get(next_index)
        }
    }
}

pub fn random_delay_seconds(min_minutes: f64, max_minutes: f64, rng: &mut impl Rng) -> f64 {
    if max_minutes <= min_minutes {
        return min_minutes * 60.0;
    }
    rng.gen_range(min_minutes..max_minutes) * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn item(id: &str) -> CatalogItem {
        CatalogItem { id: id.to_string(), original_filename: String::new(), added_at: 0.0, print_count: 0, last_printed_at: None }
    }

    #[test]
    fn sequential_advances_past_the_last_item_and_wraps() {
        let items = vec![item("a"), item("b"), item("c")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("a".into()), &mut rng).unwrap().id, "b");
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("c".into()), &mut rng).unwrap().id, "a");
        assert_eq!(choose_item(&items, Ordering::Sequential, &None, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn sequential_starts_over_if_the_last_printed_item_was_removed() {
        let items = vec![item("a"), item("b")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Sequential, &Some("no-longer-exists".into()), &mut rng).unwrap().id, "a");
    }

    #[test]
    fn random_never_returns_none_for_a_non_empty_list() {
        let items = vec![item("a")];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(choose_item(&items, Ordering::Random, &None, &mut rng).unwrap().id, "a");
    }

    #[test]
    fn choose_item_returns_none_for_an_empty_library() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert!(choose_item(&[], Ordering::Random, &None, &mut rng).is_none());
    }
}
```

- [ ] **Step 2: Write `crates/printer/src/main.rs`**

```rust
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

struct Config {
    token: String,
    state_path: PathBuf,
    complete_dir: PathBuf,
    device_path: PathBuf,
    bind_addr: String,
}

fn config_from_env() -> Config {
    Config {
        token: env::var("PRINTER_TOKEN").expect("PRINTER_TOKEN must be set"),
        state_path: PathBuf::from(env::var("PRINTER_STATE_PATH").expect("PRINTER_STATE_PATH must be set")),
        complete_dir: PathBuf::from(env::var("PRINTER_COMPLETE_DIR").expect("PRINTER_COMPLETE_DIR must be set")),
        device_path: PathBuf::from(env::var("PRINTER_DEVICE_PATH").unwrap_or_else(|_| "/dev/usb/lp0".to_string())),
        bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8082".to_string()),
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
    let url = request.url().to_string();
    let method = request.method().clone();

    if url == "/health" && method == Method::Get {
        return json_response(200, &serde_json::json!({"status": "ok"}));
    }

    let bearer = header_value(request, "Authorization");
    let access_jwt = header_value(request, "Cf-Access-Jwt-Assertion");
    if !common::auth::is_authorized(bearer, access_jwt, &config.token) {
        return json_response(401, &serde_json::json!({"error": "unauthorized"}));
    }

    match (url.as_str(), &method) {
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
    }
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
    // No local lock here: printer_device::PRINT_LOCK already serializes the
    // one device write this process ever makes, and this loop is the only
    // caller (there is no separate "print now" path in this design).
    loop {
        std::thread::sleep(Duration::from_secs(1));
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
```

- [ ] **Step 3: Run all printer crate tests**

Run: `cd crates/printer && cargo test`
Expected: previous 9 tests plus 4 new scheduler tests = 13 tests pass.

- [ ] **Step 4: Build the full workspace**

Run: `cargo build --workspace`
Expected: all four crates (`common`, `uploader`, `watcher`, `printer`) build cleanly with no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/printer/src/scheduler.rs crates/printer/src/main.rs
git commit -m "Add printer scheduler and HTTP API (status/catalog/autoprint)"
```

---

### Task 7: End-to-end local smoke test of all three services together

**Files:** none created — this task only runs the three binaries against each other locally to catch integration bugs Tasks 3-6's per-crate tests can't see (e.g., the exact filename/extension contract between uploader and watcher).

**Interfaces:** none new — exercises the full chain: `uploader` → filesystem → `watcher` → filesystem → `printer`.

- [ ] **Step 1: Start all three services against a shared temp directory tree**

```bash
mkdir -p /tmp/infinite-scroll-smoke/{partial,ready,complete,failed}
UPLOADER_TOKEN=test-token UPLOADER_PARTIAL_DIR=/tmp/infinite-scroll-smoke/partial \
  UPLOADER_READY_DIR=/tmp/infinite-scroll-smoke/ready BIND_ADDR=127.0.0.1:8081 \
  cargo run --release -p uploader &
WATCHER_READY_DIR=/tmp/infinite-scroll-smoke/ready WATCHER_COMPLETE_DIR=/tmp/infinite-scroll-smoke/complete \
  WATCHER_FAILED_DIR=/tmp/infinite-scroll-smoke/failed WATCHER_POLL_SECONDS=1 \
  cargo run --release -p watcher &
PRINTER_TOKEN=test-token PRINTER_STATE_PATH=/tmp/infinite-scroll-smoke/state.json \
  PRINTER_COMPLETE_DIR=/tmp/infinite-scroll-smoke/complete PRINTER_DEVICE_PATH=/dev/null \
  BIND_ADDR=127.0.0.1:8082 cargo run --release -p printer &
sleep 2
```

- [ ] **Step 2: Reject an unauthorized upload**

```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST http://127.0.0.1:8081/uploads --data-binary "not authorized"
```

Expected: `401`

- [ ] **Step 3: Upload a real PNG and confirm it lands in the catalog**

```bash
python3 -c "
import struct, zlib
def chunk(tag, data):
    return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
width, height = 10, 10
raw = b'\x00' + bytes([200, 20, 60] * width) 
raw = raw * height
ihdr = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
open('/tmp/infinite-scroll-smoke/test.png', 'wb').write(png)
"
curl -s -X POST http://127.0.0.1:8081/uploads \
  -H "Authorization: Bearer test-token" \
  --data-binary @/tmp/infinite-scroll-smoke/test.png
sleep 3
curl -s http://127.0.0.1:8082/catalog -H "Authorization: Bearer test-token"
```

Expected: the upload returns `{"id": "<uuid>", "filename": "<uuid>.png"}` with `201`; after the watcher's poll interval, `/catalog` returns one item with a matching `id` and `print_count: 0`.

- [ ] **Step 4: Remove the item and confirm the catalog is empty**

```bash
ID=$(curl -s http://127.0.0.1:8082/catalog -H "Authorization: Bearer test-token" | python3 -c "import json,sys; print(json.load(sys.stdin)['catalog'][0]['id'])")
curl -s -X DELETE "http://127.0.0.1:8082/catalog/$ID" -H "Authorization: Bearer test-token"
curl -s http://127.0.0.1:8082/catalog -H "Authorization: Bearer test-token"
```

Expected: the DELETE returns `{"success": true, "removed": {...}}`; the subsequent GET returns `{"catalog": []}`.

- [ ] **Step 5: Enable autoprint against `/dev/null` and confirm status reflects it**

```bash
curl -s -X POST http://127.0.0.1:8082/autoprint \
  -H "Authorization: Bearer test-token" -H "Content-Type: application/json" \
  -d '{"enabled": true, "min_minutes": 1, "max_minutes": 2, "ordering": "random"}'
curl -s http://127.0.0.1:8082/status -H "Authorization: Bearer test-token"
```

Expected: `autoprint.enabled` is `true`, `autoprint.next_print_at` is a non-null timestamp roughly 60-120 seconds in the future.

- [ ] **Step 6: Stop all three services**

```bash
kill %1 %2 %3
rm -rf /tmp/infinite-scroll-smoke
```

- [ ] **Step 7: Commit a note recording this smoke test in the README's Status section**

```bash
git add README.md
git commit -m "Document Rust services smoke-tested end to end (upload -> watch -> convert -> catalog -> autoprint)"
```

(This step's README edit happens as part of Task 12 below, not here — skip a separate commit if Task 12 hasn't run yet; this step exists to remind whoever executes Task 12 that the smoke test already passed.)

---

### Task 8: Static web frontend (`web/library/`)

**Files:**
- Create: `web/library/index.html`
- Create: `web/library/style.css`
- Create: `web/library/app.js`

**Interfaces:**
- Consumes: `printer` app's `GET /catalog`, `DELETE /catalog/{id}`, `GET /status` and `uploader`'s `POST /uploads` — called directly from the browser via `fetch()`, same-origin-independent (CORS is handled in Task 9 by having both services echo `Access-Control-Allow-Origin` — see Task 9 Step 4).
- No env-specific values are hardcoded except the two hostnames, which are read from `<meta>` tags in `index.html` so the same static files work if a hostname ever changes without a rebuild.

- [ ] **Step 1: Write `web/library/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="uploader-origin" content="https://upload.infinite-scroll.art.virtualdino.com">
<meta name="printer-origin" content="https://printer.infinite-scroll.art.virtualdino.com">
<title>Infinite Scroll — Library</title>
<link rel="stylesheet" href="style.css">
</head>
<body>
  <header class="layer layer-header">
    <h1>Infinite Scroll</h1>
    <p>Prep-time library management. Add or remove images before the installation goes offline.</p>
  </header>

  <main class="layer layer-library">
    <form id="upload-form">
      <input type="file" id="file-input" accept="image/png,image/jpeg" multiple>
      <button type="submit">Add to catalog</button>
    </form>
    <p id="upload-status" role="status"></p>
    <ul id="catalog-list"></ul>
  </main>

  <section class="layer layer-preview">
    <h2>Preview</h2>
    <p>A non-physical simulation of the next 3 scheduler picks. Nothing here is printed.</p>
    <div id="paper-strip" class="paper-strip"></div>
  </section>

  <script src="app.js"></script>
</body>
</html>
```

- [ ] **Step 2: Write `web/library/style.css` (mobile-first: unprefixed rules are the mobile/default layout; the three layers stack vertically by default, which *is* the mobile-first layout, so no media query is needed to achieve it — a `min-width` query only widens margins on larger screens)**

```css
:root {
  color-scheme: light dark;
  --ink: #1a1a1a;
  --paper: #faf7f0;
  --accent: #b23b3b;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  font-family: system-ui, sans-serif;
  background: var(--paper);
  color: var(--ink);
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
  padding: 1rem;
}

.layer { width: 100%; }

.layer-header h1 { margin: 0 0 0.25rem; font-size: 1.5rem; }
.layer-header p { margin: 0; opacity: 0.75; font-size: 0.9rem; }

.layer-library form {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  margin-bottom: 0.75rem;
}
.layer-library button { padding: 0.5rem 1rem; }
#catalog-list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.5rem; }
#catalog-list li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.5rem;
  border: 1px solid rgba(0,0,0,0.15);
  border-radius: 4px;
}
#catalog-list button { color: var(--accent); background: none; border: none; cursor: pointer; }

/* The "squiggly paper" preview: a vertical strip with a wavy clip-path on
   each edge, evoking a curl of thermal receipt paper. */
.paper-strip {
  background: white;
  color: #111;
  padding: 1rem 1.5rem;
  min-height: 6rem;
  clip-path: polygon(
    0% 2%, 5% 0%, 15% 3%, 25% 0%, 35% 3%, 45% 0%, 55% 3%, 65% 0%, 75% 3%, 85% 0%, 95% 3%, 100% 0%,
    100% 98%, 95% 100%, 85% 97%, 75% 100%, 65% 97%, 55% 100%, 45% 97%, 35% 100%, 25% 97%, 15% 100%, 5% 97%, 0% 100%
  );
  box-shadow: 0 4px 12px rgba(0,0,0,0.15);
  font-family: "Courier New", monospace;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}
.paper-strip .loop {
  border-top: 1px dashed rgba(0,0,0,0.2);
  padding-top: 0.5rem;
}
.paper-strip .loop:first-child { border-top: none; padding-top: 0; }

@media (min-width: 700px) {
  body { padding: 2rem; max-width: 700px; margin: 0 auto; }
}
```

- [ ] **Step 3: Write `web/library/app.js`**

```javascript
const uploaderOrigin = document.querySelector('meta[name="uploader-origin"]').content;
const printerOrigin = document.querySelector('meta[name="printer-origin"]').content;

const catalogList = document.getElementById("catalog-list");
const uploadForm = document.getElementById("upload-form");
const fileInput = document.getElementById("file-input");
const uploadStatus = document.getElementById("upload-status");
const paperStrip = document.getElementById("paper-strip");

async function fetchCatalog() {
  const response = await fetch(`${printerOrigin}/catalog`, { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load catalog: ${response.status}`);
  return (await response.json()).catalog;
}

async function fetchStatus() {
  const response = await fetch(`${printerOrigin}/status`, { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load status: ${response.status}`);
  return response.json();
}

async function uploadFile(file) {
  const response = await fetch(`${uploaderOrigin}/uploads`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": file.type },
    body: file,
  });
  if (!response.ok) throw new Error(`upload failed for ${file.name}: ${response.status}`);
  return response.json();
}

async function removeItem(id) {
  const response = await fetch(`${printerOrigin}/catalog/${id}`, { method: "DELETE", credentials: "include" });
  if (!response.ok) throw new Error(`failed to remove ${id}: ${response.status}`);
}

function renderCatalog(items) {
  catalogList.innerHTML = "";
  for (const item of items) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = `${item.original_filename} — printed ${item.print_count}×`;
    const removeButton = document.createElement("button");
    removeButton.textContent = "Remove";
    removeButton.addEventListener("click", async () => {
      await removeItem(item.id);
      await refreshCatalog();
    });
    li.append(label, removeButton);
    catalogList.append(li);
  }
}

/// Simulates 3 scheduler picks client-side, honoring the real ordering
/// setting, without ever calling the printer. Sequential mirrors the
/// printer's own choose_item exactly (advance past last_item_id, wrap);
/// random draws uniformly, avoiding an immediate repeat when possible.
function simulateLoops(items, ordering, lastItemId, count) {
  if (items.length === 0) return [];
  const picks = [];
  let last = lastItemId;
  for (let i = 0; i < count; i++) {
    let next;
    if (ordering === "sequential") {
      const lastIndex = items.findIndex((item) => item.id === last);
      next = items[(lastIndex + 1) % items.length];
    } else {
      const candidates = items.filter((item) => item.id !== last);
      const pool = candidates.length > 0 ? candidates : items;
      next = pool[Math.floor(Math.random() * pool.length)];
    }
    picks.push(next);
    last = next.id;
  }
  return picks;
}

function renderPreview(items, status) {
  paperStrip.innerHTML = "";
  const picks = simulateLoops(items, status.autoprint.ordering, status.autoprint.last_item_id ?? null, 3);
  if (picks.length === 0) {
    paperStrip.textContent = "The library is empty — nothing to preview yet.";
    return;
  }
  for (const pick of picks) {
    const loop = document.createElement("div");
    loop.className = "loop";
    loop.textContent = pick.original_filename;
    paperStrip.append(loop);
  }
}

async function refreshCatalog() {
  const [items, status] = await Promise.all([fetchCatalog(), fetchStatus()]);
  renderCatalog(items);
  renderPreview(items, status);
}

uploadForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const files = Array.from(fileInput.files);
  if (files.length === 0) return;
  uploadStatus.textContent = `Uploading ${files.length} file(s)…`;
  try {
    for (const file of files) {
      await uploadFile(file);
    }
    uploadStatus.textContent = "Done.";
    fileInput.value = "";
    await refreshCatalog();
  } catch (error) {
    uploadStatus.textContent = error.message;
  }
});

refreshCatalog().catch((error) => {
  uploadStatus.textContent = error.message;
});
```

- [ ] **Step 4: Commit**

```bash
git add web/library
git commit -m "Add mobile-first static library management + preview frontend"
```

---

### Task 9: CORS support in `uploader` and `printer` (needed for Task 8's browser fetches)

Task 8's frontend is served from `library.infinite-scroll.art.virtualdino.com` but calls `upload.*` and `printer.*` — different origins, so both APIs need to answer CORS preflight and echo the right headers, or every browser call in Task 8 fails silently with an opaque network error.

**Files:**
- Modify: `crates/uploader/src/main.rs` (the `handle` function)
- Modify: `crates/printer/src/main.rs` (the `handle` function)

**Interfaces:** no new public interfaces — this changes response headers only, and adds an `OPTIONS` branch to each service's routing.

- [ ] **Step 1: Add a shared CORS header helper to `crates/common/src/http.rs`**

```rust
// Append to crates/common/src/http.rs, after json_response

/// The three services and the frontend all live under
/// *.infinite-scroll.art.virtualdino.com but are different origins from a
/// browser's perspective -- every response needs this, and every OPTIONS
/// preflight needs a bare 204 carrying just these headers.
pub fn with_cors(response: Response<std::io::Cursor<Vec<u8>>>) -> Response<std::io::Cursor<Vec<u8>>> {
    response
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"https://library.infinite-scroll.art.virtualdino.com"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, DELETE, OPTIONS"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Authorization, Content-Type"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Credentials"[..], &b"true"[..]).unwrap())
}

pub fn cors_preflight_response() -> Response<std::io::Cursor<Vec<u8>>> {
    with_cors(Response::from_data(Vec::new()).with_status_code(204))
}
```

- [ ] **Step 2: Wire it into `crates/uploader/src/main.rs`'s `handle` function**

```rust
// Replace the top of `handle` in crates/uploader/src/main.rs:

fn handle(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    if *request.method() == Method::Options {
        return common::http::cors_preflight_response();
    }
    if request.url() == "/health" && *request.method() == Method::Get {
        return common::http::with_cors(json_response(200, &serde_json::json!({"status": "ok"})));
    }

    if request.url() != "/uploads" || *request.method() != Method::Post {
        return common::http::with_cors(json_response(404, &serde_json::json!({"error": "not found"})));
    }
    // ... rest of the function is unchanged except every `return json_response(...)`
    // becomes `return common::http::with_cors(json_response(...))`, and the final
    // success response likewise: `common::http::with_cors(json_response(201, ...))`
```

Apply `common::http::with_cors(...)` around every `json_response(...)` call remaining in this function (there are 6: the 401, 413 body-length, 400 read error, 413 post-read, 400 format, 201 success).

- [ ] **Step 3: Apply the same pattern to `crates/printer/src/main.rs`'s `handle` function**

```rust
// Replace the top of `handle` in crates/printer/src/main.rs:

fn handle(config: &Config, request: &mut tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    if *request.method() == Method::Options {
        return common::http::cors_preflight_response();
    }
    let url = request.url().to_string();
    let method = request.method().clone();

    if url == "/health" && method == Method::Get {
        return common::http::with_cors(json_response(200, &serde_json::json!({"status": "ok"})));
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
```

This wraps the final response once instead of every branch individually — simpler than the uploader's per-branch wrapping in Step 2; prefer this shape and go back and simplify Step 2's uploader `handle` to match (wrap once at the return site of the whole function body rather than 6 call sites) before moving on.

- [ ] **Step 4: Rebuild and rerun Task 7's smoke test with an `Origin` header to confirm CORS headers appear**

```bash
cargo build --workspace
curl -s -i -X OPTIONS http://127.0.0.1:8082/status -H "Origin: https://library.infinite-scroll.art.virtualdino.com" | grep -i "access-control"
```

Expected: `Access-Control-Allow-Origin: https://library.infinite-scroll.art.virtualdino.com` and the other two CORS headers appear in the response.

- [ ] **Step 5: Commit**

```bash
git add crates/common/src/http.rs crates/uploader/src/main.rs crates/printer/src/main.rs
git commit -m "Add CORS support so the static frontend can call uploader and printer cross-origin"
```

---

### Task 10: Cross-compile, systemd units, and the deploy script

**Files:**
- Create: `deploy/uploader.service`
- Create: `deploy/watcher.service`
- Create: `deploy/printer.service`
- Create: `deploy/deploy.sh`

**Interfaces:**
- Consumes: the three release binaries produced by cross-compiling this workspace to `aarch64-unknown-linux-gnu`.
- Produces: three running systemd services on `infinite-scroll.local`, plus the static frontend copied to a path a webserver serves (this plan does not stand up a webserver for the frontend — see the note in Step 6).

- [ ] **Step 1: Add the aarch64 cross-compilation target and linker on the dev host**

```bash
rustup target add aarch64-unknown-linux-gnu 2>&1 || nix shell nixpkgs#rustup -c rustup target add aarch64-unknown-linux-gnu
```

Add to `~/Source/infinite-scroll/.cargo/config.toml` (create the file):

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-unknown-linux-gnu-gcc"
```

- [ ] **Step 2: Cross-compile all three binaries**

```bash
cd ~/Source/infinite-scroll
nix shell nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc -c env \
  CC_aarch64_unknown_linux_gnu=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu --workspace
```

Expected: `target/aarch64-unknown-linux-gnu/release/uploader`, `.../watcher`, `.../printer` all exist. If this specific nix cross-toolchain package name doesn't resolve, run `nix search nixpkgs pkgsCross.aarch64` first to find the exact current attribute path and use that instead — don't fall back to building on the Pi without first trying at least one alternate cross-toolchain attribute name.

- [ ] **Step 3: Write the three systemd units**

```ini
# deploy/uploader.service
[Unit]
Description=Infinite Scroll uploader
After=network.target

[Service]
Type=simple
User=stags
WorkingDirectory=/home/stags/infinite-scroll
Environment=BIND_ADDR=127.0.0.1:8081
Environment=UPLOADER_PARTIAL_DIR=/var/lib/infinite-scroll/partial
Environment=UPLOADER_READY_DIR=/var/lib/infinite-scroll/ready
EnvironmentFile=/etc/infinite-scroll/uploader.env
ExecStart=/home/stags/infinite-scroll/bin/uploader
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
```

```ini
# deploy/watcher.service
[Unit]
Description=Infinite Scroll watcher/converter
After=network.target

[Service]
Type=simple
User=stags
WorkingDirectory=/home/stags/infinite-scroll
Environment=WATCHER_READY_DIR=/var/lib/infinite-scroll/ready
Environment=WATCHER_COMPLETE_DIR=/var/lib/infinite-scroll/complete
Environment=WATCHER_FAILED_DIR=/var/lib/infinite-scroll/failed
Environment=WATCHER_POLL_SECONDS=3
ExecStart=/home/stags/infinite-scroll/bin/watcher
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
```

```ini
# deploy/printer.service
[Unit]
Description=Infinite Scroll printer app (scheduler + library API)
After=network.target

[Service]
Type=simple
User=stags
SupplementaryGroups=lp
WorkingDirectory=/home/stags/infinite-scroll
Environment=BIND_ADDR=127.0.0.1:8082
Environment=PRINTER_STATE_PATH=/var/lib/infinite-scroll/printer/state.json
Environment=PRINTER_COMPLETE_DIR=/var/lib/infinite-scroll/complete
Environment=PRINTER_DEVICE_PATH=/dev/usb/lp0
EnvironmentFile=/etc/infinite-scroll/printer.env
ExecStart=/home/stags/infinite-scroll/bin/printer
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 4: Write `deploy/deploy.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "Cross-compiling..."
nix shell nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc -c env \
  CC_aarch64_unknown_linux_gnu=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu --workspace

HOST=infinite-scroll.local
BIN_DIR=target/aarch64-unknown-linux-gnu/release

echo "Copying binaries..."
ssh "$HOST" "mkdir -p /home/stags/infinite-scroll/bin"
for name in uploader watcher printer; do
  scp "$BIN_DIR/$name" "$HOST:/home/stags/infinite-scroll/bin/$name.new"
  ssh "$HOST" "mv /home/stags/infinite-scroll/bin/$name.new /home/stags/infinite-scroll/bin/$name && chmod +x /home/stags/infinite-scroll/bin/$name"
done

echo "Copying systemd units..."
scp deploy/uploader.service deploy/watcher.service deploy/printer.service "$HOST:/tmp/"
ssh "$HOST" "sudo mv /tmp/uploader.service /tmp/watcher.service /tmp/printer.service /etc/systemd/system/ && sudo systemctl daemon-reload"

echo "Restarting services..."
ssh "$HOST" "sudo systemctl enable --now uploader watcher printer && sudo systemctl restart uploader watcher printer"

echo "Health checks..."
ssh "$HOST" "curl -sf http://127.0.0.1:8081/health && echo ' uploader ok'"
ssh "$HOST" "curl -sf http://127.0.0.1:8082/health && echo ' printer ok'"
echo "Deploy complete."
```

```bash
chmod +x deploy/deploy.sh
```

- [ ] **Step 5: On the Pi, create the required directories and secret env files (one-time setup, not part of `deploy.sh` since it contains secrets)**

```bash
ssh infinite-scroll.local "mkdir -p /var/lib/infinite-scroll/{partial,ready,complete,failed,printer} && sudo mkdir -p /etc/infinite-scroll"
ssh infinite-scroll.local "sudo tee /etc/infinite-scroll/uploader.env > /dev/null" <<'EOF'
UPLOADER_TOKEN=<generate a real random token here and record it in the secrets service, not in this plan file>
EOF
ssh infinite-scroll.local "sudo tee /etc/infinite-scroll/printer.env > /dev/null" <<'EOF'
PRINTER_TOKEN=<generate a real random token here and record it in the secrets service, not in this plan file>
EOF
ssh infinite-scroll.local "sudo chmod 600 /etc/infinite-scroll/*.env"
```

- [ ] **Step 6: Run the deploy script and confirm both health checks pass**

```bash
./deploy/deploy.sh
```

Expected: both `curl` health checks print `{"status":"ok"} uploader ok` / `... printer ok`. Note: the static frontend from Task 8 still needs a place to be served from — the simplest option consistent with "small footprint" is `printer`'s own binary serving `web/library/*` as static files at `GET /` (add three lines to `printer`'s router in a follow-up, or serve it via `nginx`/`caddy` if one is already running on the Pi for something else — check before adding a new webserver dependency). This decision is deliberately left open here rather than guessed at; note it in the final report rather than silently picking one.

- [ ] **Step 7: Commit**

```bash
git add deploy/
git commit -m "Add systemd units and cross-compile/deploy script for uploader/watcher/printer"
```

---

### Task 11: Manual Cloudflare Tunnel/Access/DNS steps (NOT autonomous — write this as a document, don't attempt to execute it)

**Files:**
- Create: `docs/cloudflare-tunnel-access-setup.md`

This task's deliverable is a document, because this session has no SSH path to `pve2` (where the Tunnel connector runs) and no Cloudflare API scope beyond `zone:read`. Do not attempt any Cloudflare API calls for this task even if a token is discovered somewhere — surface that discovery and ask before using it, per this project's standing "never enable/change account-level config without an explicit decision" rule.

- [ ] **Step 1: Write `docs/cloudflare-tunnel-access-setup.md` with these exact contents**

```markdown
# Cloudflare Tunnel + Access setup for the 3 new hostnames

Three new local ports need Tunnel ingress rules and Access Applications,
matching the existing `infinite-scroll-api.virtualdino.com` / MediaWatch
pattern (human session + service token on one Access Application per
hostname).

## 1. Tunnel ingress (on pve2, wherever `cloudflared`'s config.yml lives
   for the tunnel that currently routes to infinite-scroll-api)

Add three ingress rules pointing at the Pi's LAN address (same one the
current `infinite-scroll-api` rule already targets), before the final
catch-all 404 rule:

```yaml
- hostname: upload.infinite-scroll.art.virtualdino.com
  service: http://<pi-lan-ip>:8081
- hostname: printer.infinite-scroll.art.virtualdino.com
  service: http://<pi-lan-ip>:8082
- hostname: library.infinite-scroll.art.virtualdino.com
  service: http://<pi-lan-ip>:8082
```

(`library.*` also points at the printer app's port once Task 10 Step 6's
open question is resolved by serving the static frontend from `printer`
itself — update this if a separate webserver ends up serving it instead.)

Reload the tunnel connector after editing (`cloudflared` picks up
config.yml changes on restart; check whether this specific connector is
managed by systemd and if so `systemctl restart cloudflared` on pve2, or
via the dashboard if it's a remotely-managed tunnel instead of a local
config file — check which mode this tunnel uses before assuming a local
file edit is even the right mechanism).

## 2. DNS

Each hostname needs a CNAME to the tunnel (`<tunnel-id>.cfargotunnel.com`),
same as the existing `infinite-scroll-api` record. If the tunnel is in
dashboard-managed "Public Hostname" mode, adding the hostname there
creates the DNS record automatically — check this before manually adding
CNAMEs, to avoid a conflicting duplicate record.

## 3. Access Applications (one per hostname, three total)

For each of `upload.*`, `printer.*`, `library.*`:

- Create a self-hosted Access Application for that exact hostname.
- Add two policies (both "Allow"):
  1. **Human session** — same identity provider/rule already used for
     other self-hosted apps on this account (e.g. "Emails ending in
     @<your domain>" or whatever the existing MediaWatch policy uses —
     copy it exactly rather than reinventing the rule).
  2. **Service token** — create one new Service Token per hostname (or
     reuse one across all three if that matches how `ART_ACCESS_CLIENT_ID`/
     `ART_ACCESS_CLIENT_SECRET` are already scoped — check the existing
     Access Application for `infinite-scroll-api.virtualdino.com` for the
     current convention before deciding).
- `library.*` technically only needs the human-session policy (nothing
  calls it as a service), but including the service-token policy too
  costs nothing and keeps all three Applications configured identically.

Record the resulting Client ID/Secret pairs in the secrets service (per
this project's standing policy — never leave them typed into a config
file in plaintext), named to match the existing `ART_ACCESS_CLIENT_ID`/
`ART_ACCESS_CLIENT_SECRET` convention, e.g. `UPLOAD_ACCESS_CLIENT_ID` /
`UPLOAD_ACCESS_CLIENT_SECRET`, and one pair each for `printer.*` and
`library.*` if using per-hostname tokens.

## 4. After this is done

- Confirm `curl -I https://upload.infinite-scroll.art.virtualdino.com/health`
  (with no credentials) returns Cloudflare Access's login challenge, not
  a raw 200 — proves the Tunnel + Access wiring is live before anything
  tries to use it for real.
- Update the MCP-layer secrets (Task 12) with the real hostnames and
  service-token credentials once they exist.
```

- [ ] **Step 2: Commit**

```bash
git add docs/cloudflare-tunnel-access-setup.md
git commit -m "Document the manual Cloudflare Tunnel/Access/DNS steps this session could not perform"
```

---

### Task 12: Update the MCP layer (`plugins/virtualdino` and `mcp`) to call the new services

**Files:**
- Modify: `~/Source/ops.in.net/plugins/virtualdino/src/art.rs` (rewrite `request` and every function to call the three new hostnames instead of the old `INFINITE_SCROLL_URL`/`/api/mcp/v1/*` contract)
- Modify: `~/Source/ops.in.net/plugins/virtualdino/src/lib.rs` (art routes)
- Modify: `~/Source/ops.in.net/mcp/src/art-tools.ts` (tool names/schemas: `art_add_to_catalog`, `art_list_catalog`, `art_remove_from_catalog`, `art_get_installation_status`, `art_configure_autoprint`; drop `png_file`/`image_upload_id`, keep only direct-bytes upload via a new portal-client method that itself calls the uploader — or simplest: keep `png_base64` as the sole MCP-facing input and have the Virtualdino Worker POST those bytes to the new `upload.*` service on the caller's behalf, so MCP callers never need to know the uploader endpoint exists directly)
- Delete: `~/Source/ops.in.net/mcp/src/art-upload-store.ts`, `~/Source/ops.in.net/mcp/src/art-upload-store.test.ts`
- Modify: `~/Source/ops.in.net/mcp/src/auth-handler.ts` (remove the `/art-uploads/:id` route and its import)
- Modify: `~/Source/ops.in.net/mcp/src/auth-handler.test.ts` (remove the corresponding tests)
- Modify: `~/Source/ops.in.net/mcp/src/portal-client.ts`, `~/Source/ops.in.net/mcp/src/mcp-agent.ts`, `~/Source/ops.in.net/mcp/src/tool-metadata.ts`, `~/Source/ops.in.net/mcp/src/get-started.ts`

This task is intentionally left at this level of detail rather than fully scripted line-by-line: it depends on Task 11's real hostnames and service-token secrets existing first (this is genuinely blocked on that manual step, not a scoping shortcut), and it touches a different repo with its own existing test suite (265 passing TS tests, 115 Rust tests as of the last session) that must stay green throughout. Whoever executes this task should:

- [ ] **Step 1: Re-read `~/Source/ops.in.net/plugins/virtualdino/src/art.rs` and `lib.rs` in full before changing anything** — they were already edited once this session (recent_prints/print_immediately removed, catalog list/remove added against the *old* Flask contract's `/api/mcp/v1/catalog` shape). Confirm whether that shape now matches Task 6's `printer` app's actual API (`GET /catalog` returns `{"catalog": [...]}` with `{id, original_filename, added_at, print_count, last_printed_at}` per item — compare field-by-field) before assuming it's still correct.
- [ ] **Step 2: Decide and implement how `art_add_to_catalog` gets bytes to the uploader** — either the MCP tool still accepts `png_base64` and the Virtualdino Worker forwards those bytes as a POST to `https://upload.infinite-scroll.art.virtualdino.com/uploads` with the `UPLOAD_ACCESS_CLIENT_ID`/`SECRET` + `UPLOADER_TOKEN` headers Task 11 produces, or the MCP tool is changed to return an uploader URL for the caller to PUT to directly (closer to the old upload-ticket shape, but simpler now that there's no KV ticket — just the static uploader URL plus a note that Access service-token headers are attached by the Worker, not the caller). Pick the first option unless there's a reason found during implementation to prefer the second — it keeps the MCP contract simpler for callers (still just `png_base64` in, no separate upload step) and is consistent with "MCP can upload any number of files" reading naturally as "the MCP tool call itself performs the upload," not "the MCP tool hands back yet another URL to a different service."
- [ ] **Step 3: Run both test suites and confirm they're green before and after** — `cd ~/Source/ops.in.net/mcp && npx vitest run` and `cd ~/Source/ops.in.net/plugins/virtualdino && nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test`.
- [ ] **Step 4: Update `get-started.ts`'s `ART_GUIDE`** to describe the new tool names and the simplified image-only, no-LinkedIn-post contract.
- [ ] **Step 5: Commit both repos separately**, following this session's established commit-message style (why, not just what).

---

### Task 13: Decommission the old Python app

**Files:**
- Delete: `app/`, `tests/`, `run.py`, `requirements.txt`, `requirements-dev.txt`, `pyproject.toml`, `deploy/infinite-scroll-webapp.service`, `.pytest_cache/`, `__pycache__/`
- Modify: `README.md` (rewrite entirely — see Step 3)

- [ ] **Step 1: Stop and remove the old service on the Pi**

```bash
ssh infinite-scroll.local "sudo systemctl stop infinite-scroll-webapp && sudo systemctl disable infinite-scroll-webapp"
ssh infinite-scroll.local "sudo rm -f /etc/systemd/system/infinite-scroll-webapp.service && sudo systemctl daemon-reload"
ssh infinite-scroll.local "rm -rf /home/stags/infinite-scroll-webapp"
```

Do not delete `/var/lib/infinite-scroll/webapp/backlog.sqlite3` or `/var/lib/infinite-scroll/print-ready/*.zpl` from earlier manual/webapp prints until Task 12 is confirmed working end-to-end against the new services — keep them as a rollback reference for one deploy cycle, then remove in a follow-up once confident.

- [ ] **Step 2: Delete the old Python code from the repo**

```bash
cd ~/Source/infinite-scroll
git rm -r app tests run.py requirements.txt requirements-dev.txt pyproject.toml deploy/infinite-scroll-webapp.service
rm -rf .pytest_cache __pycache__
```

- [ ] **Step 3: Rewrite `README.md`** to describe only the new Rust architecture (uploader/watcher/printer/web frontend, the three hostnames, the deploy script), removing every reference to Flask, WeasyPrint, the composer web app, and the structured LinkedIn post type. Keep the `tools/generate_post.py` reference removed too (it generated the now-dropped structured-post fixtures) — delete `tools/` and `docs/raspberry-pi-bringup.md`/`docs/validated-printing.md` only if their content has been fully superseded by this plan's `docs/cloudflare-tunnel-access-setup.md` and the ported algorithms' doc comments; otherwise keep whichever of those two docs still has non-redundant hardware bring-up detail (SD imaging, first-boot checks) that this plan didn't repeat.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Remove the Python/Flask app entirely; the Rust services replace it"
```

---

### Task 14: Final verification and cleanup

- [ ] **Step 1: Confirm the Pi is running only the new services**

```bash
ssh infinite-scroll.local "systemctl list-units --type=service --state=running | grep -i infinite-scroll"
```

Expected: `uploader.service`, `watcher.service`, `printer.service` — no `infinite-scroll-webapp.service`.

- [ ] **Step 2: Confirm no stray branches exist in the `infinite-scroll` repo**

```bash
cd ~/Source/infinite-scroll
git branch -a
git status --short
```

Expected: only the current branch (this repo's existing default, `main`, per its git log) — no other local or remote branches, nothing uncommitted.

- [ ] **Step 3: Confirm the `ops.in.net` repos (`mcp`, `plugins/virtualdino`) are also clean and on their normal branches**

```bash
cd ~/Source/ops.in.net/mcp && git status --short && git branch -a
cd ~/Source/ops.in.net/plugins/virtualdino && git status --short && git branch -a
```

Expected: clean working trees, `main`/`master` respectively, no stray branches.

- [ ] **Step 4: Push all three repos**

```bash
cd ~/Source/infinite-scroll && git push origin main   # or set up a remote first if one still doesn't exist -- check before assuming
cd ~/Source/ops.in.net/mcp && git push origin main
cd ~/Source/ops.in.net/plugins/virtualdino && git push origin master
```

If `infinite-scroll` still has no remote configured, stop here and ask whether one should be created (this is a new, meaningful decision — don't create a Forgejo repo and push to it without confirming first).
