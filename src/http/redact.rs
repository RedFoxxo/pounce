//! Keeping the token out of logs and tool output.

use std::sync::LazyLock;

use regex::Regex;

static TOKEN_PARAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)([?&](?:access_token|token)=)[^&#\s"'\\]*"#).unwrap());

/// Replaces the exact token, raw and URL-encoded. Safe on JSON: it never touches other characters.
pub fn redact_token(text: &str, token: &str) -> String {
    if token.encode_utf16().count() < 4 {
        return text.to_string();
    }
    let mut out = text.replace(token, "***");
    let encoded = super::encode_component(token);
    if encoded != token {
        out = out.replace(&encoded, "***");
    }
    out
}

/// Removes the token from a URL or log line: the exact token, plus any
/// `access_token=`/`token=` query value. Not for response bodies, where the
/// parameter pattern would rewrite user content (links) and could corrupt JSON.
pub fn redact(text: &str, token: &str) -> String {
    redact_token(&TOKEN_PARAM.replace_all(text, "${1}***"), token)
}
