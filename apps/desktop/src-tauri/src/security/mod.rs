//! Security helpers: master-key storage and log redaction.

pub mod keystore;

/// Strip query string and fragment from a URL for logging; they often
/// carry access tokens.
pub fn redact_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let had_query = u.query().is_some();
            u.set_query(None);
            u.set_fragment(None);
            let _ = u.set_password(None);
            let mut s = u.to_string();
            if had_query {
                s.push_str("?…");
            }
            s
        }
        Err(_) => "<invalid url>".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn redacts() {
        assert_eq!(
            super::redact_url("https://u:p@e.com/a?token=x#f"),
            "https://u@e.com/a?…"
        );
        assert_eq!(super::redact_url("https://e.com/a"), "https://e.com/a");
    }
}
