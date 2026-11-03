//! Small helpers for answers, query strings and URLs.

use crate::backend::Reply;

pub fn json(status: u16, value: serde_json::Value) -> Reply {
    Reply {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: value.to_string().into_bytes(),
    }
}

/// `{"error": code, "message": …}` for the plugin's page.
pub fn error(status: u16, code: &str, message: &str) -> Reply {
    json(status, serde_json::json!({ "error": code, "message": message }))
}

pub fn empty(status: u16) -> Reply {
    Reply { status, headers: Vec::new(), body: Vec::new() }
}

/// A file of the page itself.
pub fn asset(content_type: &str, body: &[u8]) -> Reply {
    Reply {
        status: 200,
        headers: vec![
            ("content-type".to_owned(), content_type.to_owned()),
            ("cache-control".to_owned(), "no-cache".to_owned()),
        ],
        body: body.to_vec(),
    }
}

/// A file id as the server writes them (a UUID): nothing that could change
/// a path it is put into.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// A parameter of a query string, decoded.
pub fn query_param(query: &str, name: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| decode(value))
        .unwrap_or_default()
}

/// Decodes `%XX` and `+` in a query value.
fn decode(value: &str) -> String {
    let hex = |byte: u8| (byte as char).to_digit(16);
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (b'+', _, _) => {
                out.push(b' ');
                i += 1;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `scheme://host[:port]` of an `http(s)://` URL, lower-cased; none for
/// anything else.
pub fn origin(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || authority.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

/// An `http(s)://` address from a setting, without the `/` at its end.
pub fn base_url(value: Option<&str>) -> Option<String> {
    let value = value?.trim().trim_end_matches('/');
    origin(value)?;
    (!value.contains(['?', '#'])).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("0b9e2f3c-1d2e-4f5a-8b7c-112233445566"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id(&"a".repeat(65)));
    }

    #[test]
    fn query() {
        assert_eq!(query_param("file=abc&origin=https%3A%2F%2Fa.b", "origin"), "https://a.b");
        assert_eq!(query_param("q=a+b", "q"), "a b");
        assert_eq!(query_param("flag&x=1", "flag"), "");
        assert_eq!(query_param("", "file"), "");
    }

    #[test]
    fn urls() {
        assert_eq!(encode("http://a:1/app/x?y=1"), "http%3A%2F%2Fa%3A1%2Fapp%2Fx%3Fy%3D1");
        assert_eq!(origin("HTTPS://Office.Example.com/browser/x"), Some("https://office.example.com".into()));
        assert_eq!(origin("http://192.168.4.131:9980"), Some("http://192.168.4.131:9980".into()));
        assert_eq!(origin("ftp://x"), None);
        assert_eq!(origin("http://user@x"), None);
        assert_eq!(base_url(Some(" http://rustcloud:8080/ ")), Some("http://rustcloud:8080".into()));
        assert_eq!(base_url(Some("https://x.example/cloud")), Some("https://x.example/cloud".into()));
        assert_eq!(base_url(Some("")), None);
        assert_eq!(base_url(Some("office.example.com")), None);
        assert_eq!(base_url(None), None);
    }
}
