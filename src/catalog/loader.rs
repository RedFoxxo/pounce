//! Reading the catalog from an instance: the index, each resource's `/meta`, the
//! unlisted history and conversion resources, and the version.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::task::Poll;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::types::{CatalogCollection, CatalogData, CatalogField, CatalogResource};
use super::{Catalog, iso_date, js_trim, locale_compare, same_name};
use crate::http::v1::V1;
use crate::http::{Logger, Query, Res, err};

/// Concurrent `/meta` requests.
pub const CONCURRENCY: usize = 8;

/// The successful response's status, which `V1::get_text` does not pass on.
const OK: u16 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    pub path: String,
    pub description: String,
}

/// `/api/v1/Index/meta` repeats the key `"ResourceMetadataDescription"` once per
/// resource, so a JSON parser keeps only the last one. Scan the raw text and
/// parse each occurrence's object on its own.
pub fn parse_index(text: &str) -> Vec<IndexEntry> {
    const KEY: &str = "\"ResourceMetadataDescription\":";
    let mut entries = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(KEY).map(|i| i + from) {
        let Some(start) = text[at + KEY.len()..].find('{').map(|i| i + at + KEY.len()) else {
            break;
        };
        let Some(end) = matching_brace(text, start) else {
            break;
        };
        from = end + 1;
        // One malformed entry is skipped rather than losing the whole index.
        let Ok(raw) = serde_json::from_str::<Value>(&text[start..=end]) else {
            continue;
        };
        let (Some(name), Some(uri)) = (non_empty(&raw, "Name"), non_empty(&raw, "Uri")) else {
            continue;
        };
        entries.push(IndexEntry {
            name: name.to_string(),
            path: path_from_uri(uri).to_string(),
            description: raw
                .get("Description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    entries
}

fn non_empty<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Byte offset of the `}` closing the object that opens at `start`.
fn matching_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 1;
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// `https://x/api/v1/UserStories/meta` or `https://x/api/v1/UserStories` → `UserStories`.
pub fn path_from_uri(uri: &str) -> &str {
    let after = uri.split("/api/v1/").nth(1).unwrap_or(uri);
    after.split(['/', '?']).next().unwrap_or(after)
}

fn xml_attributes(tag: &str) -> HashMap<String, String> {
    static ATTR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"([A-Za-z]+)="([^"]*)""#).unwrap());
    ATTR.captures_iter(tag)
        .map(|m| (m[1].to_string(), decode_xml(&m[2])))
        .collect()
}

fn decode_xml(value: &str) -> String {
    decode_char_refs(
        &value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'"),
    )
    .replace("&amp;", "&")
}

/// `&#NNN;` as `String.fromCharCode` decodes it: one UTF-16 unit each, so a
/// surrogate pair written as two references becomes one character.
pub(crate) fn decode_char_refs(text: &str) -> String {
    static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#([0-9]+);").unwrap());
    if !text.contains("&#") {
        return text.to_string();
    }
    let mut units: Vec<u16> = Vec::with_capacity(text.len());
    let mut last = 0;
    for m in REF.captures_iter(text) {
        let whole = m.get(0).expect("group 0 is the match");
        units.extend(text[last..whole.start()].encode_utf16());
        // Number(n), then ToUint16; NaN (from Infinity) becomes 0.
        let n: f64 = m[1].parse().unwrap_or(f64::INFINITY);
        units.push((n % 65536.0) as u16);
        last = whole.end();
    }
    units.extend(text[last..].encode_utf16());
    String::from_utf16_lossy(&units)
}

fn xml_bool(value: Option<&String>) -> Option<bool> {
    value.map(|v| v == "true")
}

fn xml_fields(xml: &str, section: &str) -> Value {
    static ELEMENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[A-Za-z]+\s[^>]*?/>").unwrap());
    let body = Regex::new(&format!(r"<{section}>([\s\S]*?)</{section}>"))
        .expect("section names are plain words")
        .captures(xml)
        .and_then(|c| c.get(1))
        .map_or("", |m| m.as_str());
    let fields: Vec<Value> = ELEMENT
        .find_iter(body)
        .map(|m| {
            let a = xml_attributes(m.as_str());
            let mut raw = Map::new();
            for key in ["Name", "Type"] {
                if let Some(v) = a.get(key) {
                    raw.insert(key.into(), json!(v));
                }
            }
            for key in [
                "CanSet",
                "IsRequired",
                "IsDeprecated",
                "CanAdd",
                "CanRemove",
            ] {
                if let Some(b) = xml_bool(a.get(key)) {
                    raw.insert(key.into(), json!(b));
                }
            }
            Value::Object(raw)
        })
        .collect();
    json!({ "Items": fields })
}

/// Some `/meta` endpoints (e.g. `Context/meta`) answer XML regardless of
/// `format=json`. Converts that XML to the JSON shape.
pub fn parse_meta_xml(xml: &str) -> Option<Value> {
    static ROOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<ResourceMetadataDescription\s[^>]*>").unwrap());
    static BASE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"<ResourceMetadataBaseResourceItemDescription\s[^>]*?/>").unwrap()
    });
    let root = ROOT.find(xml)?;
    let a = xml_attributes(root.as_str());
    let bases: Vec<Value> = BASE
        .find_iter(xml)
        .filter_map(|m| xml_attributes(m.as_str()).remove("Name"))
        .filter(|n| !n.is_empty())
        .map(|name| json!({ "Name": name }))
        .collect();
    let mut meta = Map::new();
    meta.insert(
        "ResourceMetadataHierarchyDescription".into(),
        json!({ "ResourceMetadataBaseResourceDescription": { "Items": bases } }),
    );
    let mut props = Map::new();
    for section in [
        "ResourceMetadataPropertiesResourceValuesDescription",
        "ResourceMetadataPropertiesResourceReferencesDescription",
        "ResourceMetadataPropertiesResourceCollectionsDescription",
    ] {
        props.insert(section.into(), xml_fields(xml, section));
    }
    meta.insert(
        "ResourceMetadataPropertiesDescription".into(),
        Value::Object(props),
    );
    for key in ["Name", "Uri", "Description"] {
        if let Some(v) = a.get(key) {
            meta.insert(key.into(), json!(v));
        }
    }
    for key in ["CanCreate", "CanUpdate", "CanDelete"] {
        if let Some(b) = xml_bool(a.get(key)) {
            meta.insert(key.into(), json!(b));
        }
    }
    Some(Value::Object(meta))
}

/// The first `n` UTF-16 units, as `slice(0, n)` cuts.
fn head(text: &str, n: usize) -> String {
    if text.len() <= n {
        return text.to_string();
    }
    let units: Vec<u16> = text.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

/// GET `/{path}/meta`, accepting JSON or the XML some endpoints insist on.
async fn fetch_meta(v1: &V1, path: &str) -> Res<Value> {
    let text = v1.get_text(&format!("{path}/meta"), Query::new()).await?;
    let body = js_trim(&text);
    if body.starts_with('{') {
        return serde_json::from_str(body).map_err(|_| {
            err(
                OK,
                format!("Unreadable JSON metadata for {path}"),
                head(body, 2000),
            )
        });
    }
    parse_meta_xml(body).ok_or_else(|| {
        err(
            OK,
            format!("Unreadable metadata for {path}"),
            head(body, 2000),
        )
    })
}

/// JavaScript truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn is_true(raw: &Value, key: &str) -> bool {
    raw.get(key) == Some(&Value::Bool(true))
}

fn raw_text(raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn field(raw: &Value) -> CatalogField {
    CatalogField {
        name: raw_text(raw, "Name"),
        type_name: raw_text(raw, "Type"),
        can_set: is_true(raw, "CanSet"),
        required: is_true(raw, "IsRequired"),
        deprecated: is_true(raw, "IsDeprecated"),
    }
}

fn collection(raw: &Value) -> CatalogCollection {
    let f = field(raw);
    CatalogCollection {
        name: f.name,
        type_name: f.type_name,
        can_set: f.can_set,
        required: f.required,
        deprecated: f.deprecated,
        can_add: is_true(raw, "CanAdd"),
        can_remove: is_true(raw, "CanRemove"),
    }
}

fn items<'v>(raw: &'v Value, pointer: &str) -> &'v [Value] {
    raw.pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Converts one `/{Resource}/meta` payload.
pub fn parse_meta(raw: &Value, fallback: &Target) -> CatalogResource {
    const PROPS: &str = "/ResourceMetadataPropertiesDescription";
    let named = |section: &str| {
        items(raw, &format!("{PROPS}/{section}/Items"))
            .iter()
            .filter(|i| truthy(i.get("Name")))
    };
    let path = match raw.get("Uri").and_then(Value::as_str) {
        Some(uri) if !uri.is_empty() => path_from_uri(uri).to_string(),
        _ => fallback.path.clone(),
    };
    CatalogResource {
        name: raw
            .get("Name")
            .and_then(Value::as_str)
            .unwrap_or(&fallback.name)
            .to_string(),
        path,
        description: raw
            .get("Description")
            .and_then(Value::as_str)
            .unwrap_or(&fallback.description)
            .to_string(),
        listed: fallback.listed,
        available: true,
        can_create: is_true(raw, "CanCreate"),
        can_update: is_true(raw, "CanUpdate"),
        can_delete: is_true(raw, "CanDelete"),
        bases: items(
            raw,
            "/ResourceMetadataHierarchyDescription/ResourceMetadataBaseResourceDescription/Items",
        )
        .iter()
        .filter_map(|i| i.get("Name").and_then(Value::as_str))
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect(),
        values: named("ResourceMetadataPropertiesResourceValuesDescription")
            .map(field)
            .collect(),
        references: named("ResourceMetadataPropertiesResourceReferencesDescription")
            .map(field)
            .collect(),
        collections: named("ResourceMetadataPropertiesResourceCollectionsDescription")
            .map(collection)
            .collect(),
    }
}

/// A resource to read `/meta` for: an index entry, or a probed unlisted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub path: String,
    pub listed: bool,
    pub description: String,
}

impl Target {
    pub fn listed(entry: &IndexEntry) -> Self {
        Target {
            name: entry.name.clone(),
            path: entry.path.clone(),
            listed: true,
            description: entry.description.clone(),
        }
    }

    pub fn unlisted(path: String) -> Self {
        Target {
            name: path.clone(),
            path,
            listed: false,
            description: String::new(),
        }
    }
}

fn unavailable(target: &Target) -> CatalogResource {
    CatalogResource {
        name: target.name.clone(),
        path: target.path.clone(),
        description: target.description.clone(),
        listed: true,
        available: false,
        can_create: false,
        can_update: false,
        can_delete: false,
        bases: Vec::new(),
        values: Vec::new(),
        references: Vec::new(),
        collections: Vec::new(),
    }
}

/// `f` over `items`, at most `limit` at a time, results in item order.
pub async fn map_limit<'a, T, R, F, Fut>(items: &'a [T], limit: usize, f: F) -> Vec<R>
where
    F: Fn(&'a T) -> Fut,
    Fut: Future<Output = R>,
{
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    let (next, out_ref, f) = (&next, &out, &f);
    let worker = move || async move {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { return };
            let r = f(item).await;
            out_ref.lock().unwrap()[i] = Some(r);
        }
    };
    let mut workers: Vec<_> = (0..limit.min(items.len()))
        .map(|_| Some(Box::pin(worker())))
        .collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for slot in workers.iter_mut() {
            if let Some(w) = slot {
                if w.as_mut().poll(cx).is_ready() {
                    *slot = None;
                } else {
                    pending = true;
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    drop(workers);
    out.into_inner().unwrap().into_iter().flatten().collect()
}

/// One retry for transient failures; a 404 means the resource does not exist here.
async fn fetch_meta_retrying(v1: &V1, path: &str) -> Res<Value> {
    match fetch_meta(v1, path).await {
        Err(e) if e.status != 404 => fetch_meta(v1, path).await,
        first => first,
    }
}

/// `/api/v1/Index/meta`: every listed resource.
pub async fn load_index(v1: &V1) -> Res<Vec<IndexEntry>> {
    let index = v1.get_text("Index/meta", Query::new()).await?;
    let entries = parse_index(&index);
    if entries.is_empty() {
        return Err(err(
            OK,
            "The resource index was empty or unreadable",
            head(&index, 2000),
        ));
    }
    Ok(entries)
}

/// Resources the index does not list but that answer `/meta` (56 on our
/// instance): every `{X}Histories` and `{X}SimpleHistories`, and `GeneralConversions`.
pub fn unlisted_targets(entries: &[IndexEntry]) -> Vec<Target> {
    let listed: HashSet<String> = entries.iter().map(|e| e.path.to_lowercase()).collect();
    entries
        .iter()
        .flat_map(|e| {
            [
                format!("{}Histories", e.name),
                format!("{}SimpleHistories", e.name),
            ]
        })
        .chain(["GeneralConversions".to_string()])
        .filter(|path| !listed.contains(&path.to_lowercase()))
        .map(Target::unlisted)
        .collect()
}

#[derive(Debug, Clone)]
pub struct LoadedResource {
    /// None when an unlisted resource does not exist here.
    pub resource: Option<Arc<CatalogResource>>,
    /// False when `/meta` failed for a reason other than 404: the answer may change, so it is not kept.
    pub is_final: bool,
    /// The resource came from the fallback (snapshot), not from the instance.
    pub from_fallback: bool,
}

/// Where a resource whose `/meta` fails comes from instead (the snapshot).
pub type Fallback = Option<fn() -> Arc<Catalog>>;

/// One resource's `/meta`. A 404 is final: a listed resource becomes
/// unavailable (e.g. SickLeave), an unlisted one does not exist. Any other
/// failure uses the fallback entry when there is one.
pub async fn load_resource(
    v1: &V1,
    target: &Target,
    log: &Logger,
    fallback: Fallback,
) -> LoadedResource {
    let gone = || target.listed.then(|| Arc::new(unavailable(target)));
    let reason = match fetch_meta_retrying(v1, &target.path).await {
        Ok(meta) => {
            return LoadedResource {
                resource: Some(Arc::new(parse_meta(&meta, target))),
                is_final: true,
                from_fallback: false,
            };
        }
        Err(e) if e.status == 404 => {
            return LoadedResource {
                resource: gone(),
                is_final: true,
                from_fallback: false,
            };
        }
        Err(e) => e.message,
    };
    let known = fallback.and_then(|baseline| baseline_entry(&baseline(), &target.path));
    if let Some(known) = known {
        log(&format!(
            "catalog: {}/meta failed ({reason}); using the snapshot entry",
            target.path
        ));
        let resource = if known.listed == target.listed {
            known
        } else {
            Arc::new(CatalogResource {
                listed: target.listed,
                ..(*known).clone()
            })
        };
        return LoadedResource {
            resource: Some(resource),
            is_final: false,
            from_fallback: true,
        };
    }
    log(&format!(
        "catalog: {}/meta failed ({reason}); {}",
        target.path,
        if target.listed {
            "marked unavailable"
        } else {
            "skipped"
        }
    ));
    LoadedResource {
        resource: gone(),
        is_final: false,
        from_fallback: false,
    }
}

/// The Targetprocess version `Context` reports, when readable.
pub async fn load_version(v1: &V1) -> Option<String> {
    let context = v1
        .get_path("Context", Query::new().with("include", "[Version]"))
        .await
        .ok()?;
    context.get("Version")?.as_str().map(str::to_string)
}

/// The entry of a path in a baseline catalog, when it is usable.
pub fn baseline_entry(baseline: &Catalog, path: &str) -> Option<Arc<CatalogResource>> {
    baseline
        .resources()
        .iter()
        .find(|r| same_name(&r.path, path) && r.available)
        .cloned()
}

/// Reads the whole catalog from the instance (the snapshot command): the index,
/// every listed resource's `/meta`, plus the unlisted ones. One broken resource
/// never fails the load; a failed index does.
pub async fn load_catalog(
    v1: &V1,
    log: &Logger,
    now_ms: i64,
    fallback: Fallback,
) -> Res<CatalogData> {
    let index = load_index(v1).await?;
    let targets: Vec<Target> = index
        .iter()
        .map(Target::listed)
        .chain(unlisted_targets(&index))
        .collect();
    let loaded = map_limit(&targets, CONCURRENCY, |t| {
        load_resource(v1, t, log, fallback)
    })
    .await;
    let mut resources: Vec<Arc<CatalogResource>> =
        loaded.into_iter().filter_map(|l| l.resource).collect();
    resources.sort_by(|a, b| locale_compare(&a.name, &b.name));
    let version = load_version(v1).await.filter(|v| !v.is_empty());
    Ok(CatalogData {
        instance: v1.http.base_url.clone(),
        version,
        captured_at: iso_date(now_ms),
        resources,
    })
}
