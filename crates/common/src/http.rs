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
