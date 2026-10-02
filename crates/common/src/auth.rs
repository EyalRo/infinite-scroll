/// A request is authorized if either:
///  - it carries the shared-secret bearer token (a service/MCP caller), or
///  - it carries a non-empty Cf-Access-Jwt-Assertion header (a browser
///    request that already passed Cloudflare Access at the edge -- these
///    services are only reachable through the Access-protected Tunnel, so
///    presence alone is sufficient; no local JWKS/JWT verification is done).
pub fn is_authorized(
    bearer_header: Option<&str>,
    access_jwt_header: Option<&str>,
    expected_token: &str,
) -> bool {
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
