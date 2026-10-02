use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug)]
pub struct ProxyError(pub String);

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Forwards an upload's raw bytes to the uploader service over a plain
/// loopback TCP connection, authenticating with `token` (the uploader's own
/// bearer token) rather than a Cloudflare Access JWT -- this call never
/// goes through Cloudflare, it's a direct process-to-process hop on the
/// same host. Exists so the browser can POST to same-origin `/uploads` on
/// `printer`/`library` instead of the genuinely separate `uploader`
/// origin, which needs its own interactive Cloudflare Access login before
/// a background `fetch()` to it will ever succeed -- that kept breaking
/// real uploads whenever a browser profile's session for that hostname
/// expired or never existed.
pub fn forward_upload(uploader_addr: &str, token: &str, body: &[u8]) -> Result<(u16, Vec<u8>), ProxyError> {
    let mut stream = TcpStream::connect(uploader_addr).map_err(|error| ProxyError(format!("failed to connect to uploader: {error}")))?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

    let request = format!(
        "POST /uploads HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(|error| ProxyError(format!("failed to write request: {error}")))?;
    stream.write_all(body).map_err(|error| ProxyError(format!("failed to write body: {error}")))?;

    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes).map_err(|error| ProxyError(format!("failed to read response: {error}")))?;

    parse_http_response(&response_bytes)
}

/// Parses a minimal HTTP/1.1 response: status code plus body, trusting
/// `Content-Length` (uploader's own responses are always fixed-length
/// `Response::from_data`, never chunked) rather than relying on the
/// connection closing, which is more robust regardless of the peer's
/// exact keep-alive behavior.
fn parse_http_response(bytes: &[u8]) -> Result<(u16, Vec<u8>), ProxyError> {
    let header_end = find_subslice(bytes, b"\r\n\r\n").ok_or_else(|| ProxyError("malformed response: no header terminator".into()))?;
    let header_text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| ProxyError("malformed response headers".into()))?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or_else(|| ProxyError("empty response".into()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| ProxyError(format!("malformed status line: {status_line}")))?;

    let content_length: usize = lines
        .find_map(|line| line.to_ascii_lowercase().starts_with("content-length:").then(|| line[line.find(':').unwrap() + 1..].trim().to_string()))
        .ok_or_else(|| ProxyError("response missing Content-Length".into()))?
        .parse()
        .map_err(|_| ProxyError("invalid Content-Length".into()))?;

    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available < content_length {
        return Err(ProxyError(format!("response body shorter than Content-Length ({available} < {content_length})")));
    }
    Ok((status, bytes[body_start..body_start + content_length].to_vec()))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_response_extracts_status_and_exactly_content_length_bytes() {
        let raw = b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}TRAILING-GARBAGE";
        let (status, body) = parse_http_response(raw).unwrap();
        assert_eq!(status, 201);
        assert_eq!(body, b"{\"ok\":true}");
    }

    #[test]
    fn parse_http_response_rejects_malformed_input() {
        assert!(parse_http_response(b"not an http response").is_err());
    }

    #[test]
    fn parse_http_response_rejects_a_body_shorter_than_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\ntoo short";
        assert!(parse_http_response(raw).is_err());
    }

    #[test]
    fn forward_upload_relays_status_body_and_authorization_through_a_real_local_server() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr,
            _ => panic!("expected an IP listen address"),
        };

        let handle = std::thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let mut body = Vec::new();
            request.as_reader().read_to_end(&mut body).unwrap();
            let auth = request
                .headers()
                .iter()
                .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case("Authorization"))
                .map(|header| header.value.as_str().to_string());
            assert_eq!(auth.as_deref(), Some("Bearer test-token"));
            assert_eq!(body, b"fake-image-bytes");

            let response_body = br#"{"id":"abc123"}"#.to_vec();
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            request.respond(tiny_http::Response::from_data(response_body).with_status_code(201).with_header(header)).unwrap();
        });

        let (status, body) = forward_upload(&addr.to_string(), "test-token", b"fake-image-bytes").unwrap();
        handle.join().unwrap();

        assert_eq!(status, 201);
        assert_eq!(body, br#"{"id":"abc123"}"#);
    }

    #[test]
    fn forward_upload_reports_an_error_when_nothing_is_listening() {
        // Port 0 as a *target* (rather than a bind address) never has a
        // listener -- connect() fails immediately instead of hanging.
        let result = forward_upload("127.0.0.1:0", "test-token", b"data");
        assert!(result.is_err());
    }
}
