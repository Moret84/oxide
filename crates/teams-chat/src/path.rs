use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Same character set as JavaScript's `encodeURIComponent`, which the Teams
/// client applies to every conversation, thread and message id in a path.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub(crate) fn enc(segment: &str) -> String {
    utf8_percent_encode(segment, URI_COMPONENT).to_string()
}

/// Builds a query string, encoding each value like `encodeURIComponent`.
pub(crate) fn query(params: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = params
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", enc(v))))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(
            enc("19:abc@thread.v2;messageid=123"),
            "19%3Aabc%40thread.v2%3Bmessageid%3D123"
        );
    }

    #[test]
    fn query_skips_missing_values() {
        assert_eq!(
            query(&[
                ("a", Some("1".into())),
                ("b", None),
                ("c", Some("x y".into()))
            ]),
            "?a=1&c=x%20y"
        );
        assert_eq!(query(&[("a", None)]), "");
    }
}
