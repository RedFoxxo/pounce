//! The single place where pounce talks to Targetprocess: adds the token, logs the
//! redacted request to stderr, and turns every outcome into a `Res`.

pub mod redact;
#[cfg(test)]
mod tests;
pub mod v1;
pub mod v2;

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use redact::{redact, redact_token};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// A failed request. HTTP failures are values, never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpErr {
    /// HTTP status, or 0 when no response was received (network error, timeout).
    pub status: u16,
    /// One-line summary: Targetprocess's own `Message` when it sent one.
    pub message: String,
    /// Raw response body (redacted), or the reason no response was received.
    pub body: String,
    /// `METHOD redacted-url`, for diagnostics.
    pub request: Option<String>,
}

pub fn err(status: u16, message: impl Into<String>, body: impl Into<String>) -> TpErr {
    TpErr {
        status,
        message: message.into(),
        body: body.into(),
        request: None,
    }
}

pub type Res<T> = Result<T, TpErr>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormField {
    Text {
        name: String,
        value: String,
    },
    File {
        name: String,
        filename: String,
        content_type: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    Json(Value),
    /// Multipart form (attachment upload).
    Form(Vec<FormField>),
}

/// Query parameters in order; empty and missing values are left out when sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query(pub Vec<(String, String)>);

impl Query {
    pub fn new() -> Self {
        Query(Vec::new())
    }

    /// Sets `key`, replacing an earlier value in place (as an object spread does).
    pub fn set(mut self, key: &str, value: Option<impl ToString>) -> Self {
        let value = value.map(|v| v.to_string()).unwrap_or_default();
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key.to_string(), value)),
        }
        self
    }

    /// `set` for a value that is always present.
    pub fn with(self, key: &str, value: impl ToString) -> Self {
        self.set(key, Some(value))
    }

    /// `other` on top of `self`.
    pub fn merge(mut self, other: &Query) -> Self {
        for (k, v) in &other.0 {
            self = self.with(k, v);
        }
        self
    }
}

/// A request below the instance root.
#[derive(Debug, Clone)]
pub struct Spec {
    pub method: Method,
    /// Path below the instance root, starting with `/`, already encoded.
    pub path: String,
    pub query: Query,
    pub body: Body,
    /// The raw body as a JSON string instead of parsed JSON.
    pub expect_text: bool,
}

impl Spec {
    pub fn new(method: Method, path: impl Into<String>, query: Query) -> Self {
        Spec {
            method,
            path: path.into(),
            query,
            body: Body::Empty,
            expect_text: false,
        }
    }

    pub fn json(mut self, value: Value) -> Self {
        self.body = Body::Json(value);
        self
    }

    pub fn text(mut self) -> Self {
        self.expect_text = true;
        self
    }
}

/// What goes over the wire; the transport adds nothing but the body encoding.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    /// The body, or why reading it failed.
    pub body: Result<String, String>,
}

/// Sends one request without following redirects. `Err` when no response arrived,
/// as `Name: message` (e.g. `TimeoutError: …`).
pub trait Transport: Send + Sync {
    fn send(&self, request: Request, timeout: Duration) -> BoxFuture<'_, Result<Response, String>>;
}

/// The network, through reqwest.
pub struct Network {
    client: reqwest::Client,
}

impl Network {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the HTTP client builds");
        Network { client }
    }
}

impl Transport for Network {
    fn send(&self, request: Request, timeout: Duration) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
                Method::Delete => reqwest::Method::DELETE,
            };
            let mut builder = self.client.request(method, &request.url).timeout(timeout);
            for (k, v) in &request.headers {
                builder = builder.header(*k, v);
            }
            builder = match request.body {
                Body::Empty => builder,
                Body::Json(value) => builder.body(value.to_string()),
                Body::Form(fields) => {
                    let mut form = reqwest::multipart::Form::new();
                    for f in fields {
                        form = match f {
                            FormField::Text { name, value } => form.text(name, value),
                            FormField::File {
                                name,
                                filename,
                                content_type,
                                bytes,
                            } => {
                                let part = reqwest::multipart::Part::bytes(bytes)
                                    .file_name(filename)
                                    .mime_str(&content_type)
                                    .map_err(|e| format!("TypeError: {e}"))?;
                                form.part(name, part)
                            }
                        };
                    }
                    builder.multipart(form)
                }
            };
            let response = builder.send().await.map_err(reason_of)?;
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let body = response.text().await.map_err(|e| e.to_string());
            Ok(Response {
                status,
                location,
                body,
            })
        })
    }
}

/// `Name: message`, as the TypeScript build reported fetch failures.
fn reason_of(error: reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        message = format!("{message}: {cause}");
        source = cause.source();
    }
    if error.is_timeout() {
        format!("TimeoutError: {message}")
    } else {
        format!("TypeError: {message}")
    }
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const BODY_CAP: usize = 4000;

pub struct Http {
    pub base_url: String,
    token: String,
    transport: Box<dyn Transport>,
    log: Logger,
    timeout: Duration,
}

/// `encodeURIComponent`'s safe set: everything else is percent-encoded.
pub const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn encode_component(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, COMPONENT).to_string()
}

impl Http {
    pub fn new(
        base_url: String,
        token: String,
        transport: Box<dyn Transport>,
        log: Logger,
    ) -> Self {
        Http {
            base_url,
            token,
            transport,
            log,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn url(&self, path: &str, query: &Query) -> String {
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in query.0.iter().filter(|(_, v)| !v.is_empty()) {
            if k != "access_token" {
                params.append_pair(k, v);
            }
        }
        params.append_pair("access_token", &self.token);
        format!("{}{path}?{}", self.base_url, params.finish())
    }

    pub fn redact(&self, text: &str) -> String {
        redact(text, &self.token)
    }

    pub async fn request(&self, spec: Spec) -> Res<Value> {
        let url = self.url(&spec.path, &spec.query);
        let decoded = percent_encoding::percent_decode_str(&url)
            .decode_utf8()
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.clone());
        let shown = format!("{} {}", spec.method.as_str(), self.redact(&decoded));
        (self.log)(&shown);

        let mut headers = vec![("Accept", "application/json".to_string())];
        if matches!(spec.body, Body::Json(_)) {
            headers.push(("Content-Type", "application/json".to_string()));
        }
        let request = Request {
            method: spec.method,
            url,
            headers,
            body: spec.body,
        };
        let failed = |status: u16, message: String, body: String| TpErr {
            status,
            message,
            body,
            request: Some(shown.clone()),
        };

        let response = match self.transport.send(request, self.timeout).await {
            Ok(r) => r,
            Err(reason) => {
                let unknown = if spec.method == Method::Get {
                    String::new()
                } else {
                    format!(
                        "; the {} may or may not have been applied, so check before retrying",
                        spec.method.as_str()
                    )
                };
                let reason = self.redact(&reason);
                return Err(failed(
                    0,
                    format!("No response from Targetprocess ({reason}){unknown}"),
                    reason,
                ));
            }
        };
        let ok = (200..300).contains(&response.status);
        let text = match response.body {
            Ok(text) => text,
            Err(reason) => {
                let unknown = if spec.method == Method::Get {
                    String::new()
                } else {
                    format!(
                        "; Targetprocess answered {}{}: check before retrying",
                        response.status,
                        if ok {
                            ", so the write was most likely applied"
                        } else {
                            ""
                        }
                    )
                };
                return Err(failed(
                    response.status,
                    format!(
                        "Could not read the response body ({}){unknown}",
                        self.redact(&reason)
                    ),
                    String::new(),
                ));
            }
        };
        // Bodies: only the exact token is removed. The query-parameter pattern would rewrite user content (links) and can corrupt JSON.
        let text = redact_token(&text, &self.token);

        if (300..400).contains(&response.status) {
            let location = self.redact(response.location.as_deref().unwrap_or(""));
            let target = if location.is_empty() {
                "another page".to_string()
            } else {
                location
            };
            return Err(failed(
                response.status,
                format!(
                    "Targetprocess redirected to {target}; the endpoint or method is not supported"
                ),
                cap_body(&text),
            ));
        }
        if !ok {
            return Err(failed(
                response.status,
                error_message(response.status, &text),
                cap_body(&text),
            ));
        }
        if spec.expect_text {
            return Ok(Value::String(text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| {
            failed(
                response.status,
                format!(
                    "Expected JSON from Targetprocess but got {}",
                    describe_body(&text)
                ),
                cap_body(&text),
            )
        })
    }
}

/// Error bodies go into tool output; an HTML error page must not flood it.
fn cap_body(text: &str) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= BODY_CAP {
        return text.to_string();
    }
    format!(
        "{}… ({} more characters)",
        String::from_utf16_lossy(&units[..BODY_CAP]),
        units.len() - BODY_CAP
    )
}

fn describe_body(text: &str) -> &'static str {
    let head: String = text
        .trim_start()
        .chars()
        .take(15)
        .collect::<String>()
        .to_lowercase();
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        "an HTML page"
    } else if head.starts_with('<') {
        "XML"
    } else {
        "a non-JSON body"
    }
}

/// Extracts Targetprocess's own message from a JSON or XML error body.
pub fn error_message(status: u16, text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with('{')
        && let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(trimmed)
    {
        for key in ["Message", "message", "error", "Error"] {
            if let Some(Value::String(v)) = parsed.get(key)
                && !v.is_empty()
            {
                return format!("{status}: {v}");
            }
        }
    }
    if let Some(m) = regex::Regex::new(r"<Message>([\s\S]*?)</Message>")
        .unwrap()
        .captures(trimmed)
        .and_then(|c| c.get(1))
        .filter(|m| !m.as_str().is_empty())
    {
        return format!("{status}: {}", m.as_str().trim());
    }
    if trimmed.starts_with('<') {
        return format!(
            "{status}: Targetprocess returned {}",
            describe_body(trimmed)
        );
    }
    let units: Vec<u16> = trimmed
        .split('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .take(200)
        .collect();
    let first_line = String::from_utf16_lossy(&units);
    if first_line.is_empty() {
        format!("{status}: request failed with no body")
    } else {
        format!("{status}: {first_line}")
    }
}
