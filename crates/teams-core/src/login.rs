//! Microsoft sign-in return detection, shared with the macOS login window.
use url::Url;

/// The exact Teams return URL: stopping there keeps Teams' script from consuming the code before the core redeems it.
pub fn is_callback(target: &str) -> bool {
    Url::parse(target).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("teams.microsoft.com")
            && url.port_or_known_default() == Some(443)
            && url.path() == "/v2"
            && (url.query().is_some() || url.fragment().is_some())
    })
}
/// Pages the login window may load: Microsoft identity and federation pages, never plain HTTP.
pub fn is_allowed(target: &str) -> bool {
    target == "about:blank" || Url::parse(target).is_ok_and(|u| u.scheme() == "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intercepts_only_the_exact_teams_return() {
        assert!(is_callback(
            "https://teams.microsoft.com/v2#code=abc&state=xyz"
        ));
        assert!(is_callback(
            "https://teams.microsoft.com/v2?error=access_denied"
        ));
        assert!(!is_callback("https://teams.microsoft.com/v2"));
        assert!(!is_callback(
            "https://teams.microsoft.com.attacker.example/v2#code=abc"
        ));
        assert!(!is_callback("http://teams.microsoft.com/v2#code=abc"));
        assert!(!is_callback("https://login.microsoftonline.com/common/"));
        assert!(is_allowed("https://login.microsoftonline.com/common/"));
        assert!(!is_allowed("http://login.microsoftonline.com/common/"));
        assert!(!is_allowed("javascript:alert(1)"));
    }
}
