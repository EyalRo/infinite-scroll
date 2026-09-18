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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_response_sets_the_content_type_and_status() {
        let response = json_response(201, &serde_json::json!({"ok": true}));
        assert_eq!(response.status_code().0, 201);
    }

    #[test]
    fn with_cors_sets_the_exact_expected_header_values() {
        let response = with_cors(json_response(200, &serde_json::json!({})));
        let expected: &[(&str, &str)] = &[
            ("Access-Control-Allow-Origin", "https://library.infinite-scroll.art.virtualdino.com"),
            ("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS"),
            ("Access-Control-Allow-Headers", "Authorization, Content-Type"),
            ("Access-Control-Allow-Credentials", "true"),
        ];
        for (name, value) in expected {
            let found = response.headers().iter().find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name));
            assert_eq!(found.map(|header| header.value.as_str()), Some(*value), "missing or wrong value for header {name}");
        }
    }
}
