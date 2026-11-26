//! Settings from the environment, all checked at once: a bad setup fails at
//! startup with one message listing every problem.

use std::collections::HashMap;

use url::Url;

use crate::tools::{TOOLSETS, Toolset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Instance root without a trailing slash, e.g. `https://mamami.tpondemand.com`.
    pub base_url: String,
    pub token: String,
    pub default_project_id: Option<i64>,
    pub default_team_id: Option<i64>,
    /// Tool groups to expose; every group when unset.
    pub toolsets: Option<Vec<Toolset>>,
}

impl Config {
    /// A configuration for tests and tools that need no environment.
    pub fn new(base_url: &str, token: &str) -> Self {
        Config {
            base_url: base_url.to_string(),
            token: token.to_string(),
            default_project_id: None,
            default_team_id: None,
            toolsets: None,
        }
    }
}

fn optional_id(
    env: &HashMap<String, String>,
    key: &str,
    problems: &mut Vec<String>,
) -> Option<i64> {
    let raw = env.get(key).map(|v| v.trim()).filter(|v| !v.is_empty())?;
    match raw.parse::<i64>() {
        Ok(v) if raw.bytes().all(|b| b.is_ascii_digit()) && v > 0 && v <= 9_007_199_254_740_991 => {
            Some(v)
        }
        _ => {
            problems.push(format!(
                "{key} must be a positive numeric id, got \"{raw}\""
            ));
            None
        }
    }
}

fn toolsets(env: &HashMap<String, String>, problems: &mut Vec<String>) -> Option<Vec<Toolset>> {
    let raw = env
        .get("POUNCE_TOOLSETS")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())?;
    let known = TOOLSETS.map(Toolset::name).join(", ");
    let names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        problems.push(format!(
            "POUNCE_TOOLSETS names no toolset (known: {known}); unset it to register them all"
        ));
        return None;
    }
    let unknown: Vec<&str> = names
        .iter()
        .filter(|n| !TOOLSETS.iter().any(|t| t.name() == n.as_str()))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        problems.push(format!(
            "POUNCE_TOOLSETS has unknown toolsets: {} (known: {known})",
            unknown.join(", ")
        ));
        return None;
    }
    Some(
        TOOLSETS
            .into_iter()
            .filter(|t| names.iter().any(|n| n == t.name()))
            .collect(),
    )
}

fn base_url(raw: &str, problems: &mut Vec<String>) -> String {
    let Ok(url) = Url::parse(raw) else {
        problems.push(format!("TP_BASE_URL is not a valid URL: \"{raw}\""));
        return String::new();
    };
    let host = url.host_str().unwrap_or("");
    let local = ["localhost", "127.0.0.1", "[::1]"].contains(&host);
    let api = regex::Regex::new(r"(?i)/api(/|$)").unwrap();
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        problems.push(format!(
            "TP_BASE_URL must be an https URL (the token travels in the query string), got \"{raw}\""
        ));
    } else if api.is_match(url.path()) {
        problems.push(format!(
            "TP_BASE_URL must be the instance root without /api/..., got \"{raw}\""
        ));
    } else if url.query().is_some() || url.fragment().is_some() {
        problems.push("TP_BASE_URL must not contain a query string or fragment".into());
    } else {
        let origin = url.origin().ascii_serialization();
        return format!("{origin}{}", url.path())
            .trim_end_matches('/')
            .to_string();
    }
    String::new()
}

/// Reads and validates the environment. `Err` lists every problem at once.
pub fn load_config(env: &HashMap<String, String>) -> Result<Config, String> {
    let mut problems = Vec::new();
    let base = match env
        .get("TP_BASE_URL")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        None => {
            problems
                .push("TP_BASE_URL is required, e.g. https://yourcompany.tpondemand.com".into());
            String::new()
        }
        Some(raw) => base_url(raw, &mut problems),
    };
    let token = env
        .get("TP_TOKEN")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    if token.is_empty() {
        problems.push("TP_TOKEN is required (a Targetprocess access token)".into());
    }
    let default_project_id = optional_id(env, "TP_DEFAULT_PROJECT_ID", &mut problems);
    let default_team_id = optional_id(env, "TP_DEFAULT_TEAM_ID", &mut problems);
    let sets = toolsets(env, &mut problems);
    if !problems.is_empty() {
        return Err(format!(
            "Invalid pounce configuration:\n- {}",
            problems.join("\n- ")
        ));
    }
    Ok(Config {
        base_url: base,
        token,
        default_project_id,
        default_team_id,
        toolsets: sets,
    })
}
