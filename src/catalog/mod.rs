//! The instance's metadata: which resources exist, their fields, references and
//! collections, and what may be written. Read lazily from the instance, with the
//! committed `snapshot.json` standing in when it cannot be read.

pub mod loader;
pub mod snapshot;
pub mod types;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde_json::Value;
use tokio::sync::OnceCell;

use crate::http::Logger;
use crate::http::v1::V1;
use crate::tools::Clock;
use loader::{CONCURRENCY, IndexEntry, LoadedResource, Target};
pub use types::{CatalogCollection, CatalogData, CatalogResource, Member};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Snapshot,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Live => "live",
            Source::Snapshot => "snapshot",
        }
    }
}

/// A found value, or why there is none (with suggestions).
pub type Lookup<T> = Result<T, String>;

/// JavaScript's `trim()`: its whitespace includes U+FEFF but not U+0085.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// JavaScript's `\s` and `trim()` whitespace.
pub(crate) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Equal after `toLowerCase()`, without allocating.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

/// `localeCompare` for resource and field names: case-insensitive, lower case
/// before upper case on ties.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
    fold(a).cmp(&fold(b)).then_with(|| {
        a.chars()
            .zip(b.chars())
            .find(|(x, y)| x != y)
            .map_or(Ordering::Equal, |(x, _)| {
                if x.is_lowercase() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            })
    })
}

/// `new Date(ms).toISOString()`.
pub(crate) fn iso_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

/// Closest names first: prefix/substring matches, then small edit distance.
pub fn suggestions<S: AsRef<str>>(input: &str, names: &[S], max: usize) -> Vec<String> {
    let q = input.to_lowercase();
    let q_units: Vec<u16> = q.encode_utf16().collect();
    let threshold = 2.max(q_units.len() / 3);
    let mut row = Vec::new();
    let mut scored: Vec<(i64, &str)> = names
        .iter()
        .filter_map(|name| {
            let name = name.as_ref();
            let n = name.to_lowercase();
            let n_units: Vec<u16> = n.encode_utf16().collect();
            let d = distance(&q_units, &n_units, &mut row);
            let contains = n.contains(&q);
            let score = d as i64 - if n.starts_with(&q) { 3 } else { 0 } - i64::from(contains);
            (d <= threshold || contains).then_some((score, name))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| locale_compare(a.1, b.1)));
    scored
        .into_iter()
        .take(max)
        .map(|(_, n)| n.to_string())
        .collect()
}

/// Levenshtein distance over UTF-16 units, as the TypeScript build counted.
fn distance(a: &[u16], b: &[u16], row: &mut Vec<usize>) -> usize {
    row.clear();
    row.extend(0..=b.len());
    for i in 1..=a.len() {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let tmp = row[j];
            row[j] = (row[j] + 1)
                .min(row[j - 1] + 1)
                .min(prev + usize::from(a[i - 1] != b[j - 1]));
            prev = tmp;
        }
    }
    row[b.len()]
}

pub struct Catalog {
    pub data: CatalogData,
    pub source: Source,
    /// Every resource name the instance lists, when `data` holds only the resources loaded so far.
    names: Option<Vec<String>>,
    /// Index into `data.resources` by lower-case name and path.
    by_key: HashMap<String, usize>,
}

impl Catalog {
    pub fn new(data: CatalogData, source: Source, names: Option<Vec<String>>) -> Self {
        let mut by_key = HashMap::with_capacity(data.resources.len() * 2);
        for (i, r) in data.resources.iter().enumerate() {
            by_key.insert(r.name.to_lowercase(), i);
            by_key.insert(r.path.to_lowercase(), i);
        }
        Catalog {
            data,
            source,
            names,
            by_key,
        }
    }

    pub fn resources(&self) -> &[Arc<CatalogResource>] {
        &self.data.resources
    }

    /// By singular name (`UserStory`) or plural path (`UserStories`), case-insensitive.
    pub fn find(&self, name_or_path: &str) -> Option<&Arc<CatalogResource>> {
        self.by_key
            .get(&js_trim(name_or_path).to_lowercase())
            .map(|&i| &self.data.resources[i])
    }

    pub fn resource(&self, name_or_path: &str) -> Lookup<&Arc<CatalogResource>> {
        match self.find(name_or_path) {
            Some(found) if found.available => Ok(found),
            Some(found) => Err(format!(
                "Resource \"{}\" is listed by the instance but its metadata is unavailable (/meta failed), so it cannot be used.",
                found.name
            )),
            None => {
                let close = match &self.names {
                    Some(names) => suggestions(name_or_path, names, 8),
                    None => {
                        let names: Vec<&str> = self
                            .resources()
                            .iter()
                            .filter(|r| r.available)
                            .map(|r| r.name.as_str())
                            .collect();
                        suggestions(name_or_path, &names, 8)
                    }
                };
                Err(format!(
                    "Unknown resource \"{name_or_path}\".{} Use read_meta without a resource to list them all.",
                    did_you_mean(&close)
                ))
            }
        }
    }

    /// Any value, reference or collection of the resource, by name (case-insensitive).
    pub fn member<'r>(&self, resource: &'r CatalogResource, name: &str) -> Option<Member<'r>> {
        let hit = |member: &str| same_name(member, name);
        resource
            .values
            .iter()
            .find(|f| hit(&f.name))
            .map(Member::Value)
            .or_else(|| {
                resource
                    .references
                    .iter()
                    .find(|f| hit(&f.name))
                    .map(Member::Reference)
            })
            .or_else(|| {
                resource
                    .collections
                    .iter()
                    .find(|c| hit(&c.name))
                    .map(Member::Collection)
            })
    }

    pub fn collection<'r>(
        &self,
        resource: &'r CatalogResource,
        name: &str,
    ) -> Lookup<&'r CatalogCollection> {
        if let Some(found) = resource
            .collections
            .iter()
            .find(|c| same_name(&c.name, name))
        {
            return Ok(found);
        }
        let names: Vec<&str> = resource
            .collections
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        let close = suggestions(name, &names, 8);
        let all = names.join(", ");
        Err(format!(
            "{} has no collection \"{name}\".{} Collections: {}.",
            resource.name,
            did_you_mean(&close),
            if all.is_empty() { "(none)" } else { &all }
        ))
    }
}

/// ` Did you mean: a, b?`, or nothing.
pub(crate) fn did_you_mean(close: &[String]) -> String {
    if close.is_empty() {
        String::new()
    } else {
        format!(" Did you mean: {}?", close.join(", "))
    }
}

/// The committed baseline catalog (`src/catalog/snapshot.json`), parsed once.
pub fn snapshot() -> Arc<Catalog> {
    static SNAPSHOT: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
        let raw: Value = serde_json::from_str(include_str!("snapshot.json"))
            .expect("src/catalog/snapshot.json is valid JSON");
        Arc::new(Catalog::new(
            CatalogData::from_json(&raw),
            Source::Snapshot,
            None,
        ))
    });
    SNAPSHOT.clone()
}

/// The resources a tool needs, by name or path (`UserStory`, `UserStoryHistories`), or all of them.
pub enum Request {
    Names(Vec<String>),
    All,
}

/// One load shared by concurrent callers. A result `settle` does not keep is
/// dropped as soon as it arrives, so the next call loads again.
struct Flight<T> {
    slot: Mutex<Option<Arc<OnceCell<T>>>>,
}

impl<T> Default for Flight<T> {
    fn default() -> Self {
        Flight {
            slot: Mutex::new(None),
        }
    }
}

impl<T: Clone> Flight<T> {
    async fn run<F: Future<Output = T>>(
        &self,
        load: impl FnOnce() -> F,
        settle: impl FnOnce(&T) -> bool,
    ) -> T {
        let cell = self
            .slot
            .lock()
            .unwrap()
            .get_or_insert_with(Default::default)
            .clone();
        cell.get_or_init(|| async {
            let value = load().await;
            if !settle(&value) {
                let mut slot = self.slot.lock().unwrap();
                if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                    *slot = None;
                }
            }
            value
        })
        .await
        .clone()
    }
}

/// Reads the instance's metadata lazily: nothing at startup, then the index and
/// each resource's `/meta` the first time a tool asks for it (single-flight:
/// concurrent callers share one request). Answers are kept for the life of the
/// process; a failed request is not kept, so the next call tries again, and in
/// the meantime the committed snapshot stands in.
pub struct Provider(Inner);

enum Inner {
    /// Tests: always this catalog, no requests.
    #[cfg(test)]
    Fixed(Arc<Catalog>),
    Live(Box<Live>),
}

struct Live {
    v1: V1,
    log: Logger,
    clock: Clock,
    index: Flight<Option<Arc<Vec<IndexEntry>>>>,
    version: Flight<Option<String>>,
    /// In-flight and settled loads, by lower-case path.
    loads: Mutex<HashMap<String, Arc<Flight<LoadedResource>>>>,
    /// Settled final answers, by lower-case path.
    known: Mutex<HashMap<String, Arc<CatalogResource>>>,
}

impl Provider {
    pub fn live(v1: V1, log: Logger, clock: Clock) -> Self {
        Provider(Inner::Live(Box::new(Live {
            v1,
            log,
            clock,
            index: Flight::default(),
            version: Flight::default(),
            loads: Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
        })))
    }

    #[cfg(test)]
    pub fn fixed(catalog: Arc<Catalog>) -> Self {
        Provider(Inner::Fixed(catalog))
    }

    /// A catalog holding at least the requested resources (and every one loaded before).
    pub async fn get(&self, request: &Request) -> Arc<Catalog> {
        match &self.0 {
            #[cfg(test)]
            Inner::Fixed(catalog) => catalog.clone(),
            Inner::Live(live) => live.get(request).await,
        }
    }
}

impl Live {
    async fn get(&self, request: &Request) -> Arc<Catalog> {
        let Some(entries) = self.load_index().await else {
            (self.log)(
                "catalog: the resource index could not be read; using the snapshot for this call",
            );
            return snapshot();
        };
        let targets: Vec<Target> = match request {
            Request::All => entries
                .iter()
                .map(Target::listed)
                .chain(loader::unlisted_targets(&entries))
                .collect(),
            Request::Names(names) => names.iter().filter_map(|n| target(n, &entries)).collect(),
        };
        let loaded = loader::map_limit(&targets, CONCURRENCY, |t| self.load_resource(t)).await;
        let mut resources = self.known.lock().unwrap().clone();
        for r in loaded.iter().filter_map(|l| l.resource.as_ref()) {
            resources.insert(r.path.to_lowercase(), r.clone());
        }
        let mut resources: Vec<Arc<CatalogResource>> = resources.into_values().collect();
        resources.sort_by(|a, b| locale_compare(&a.name, &b.name));
        let version = match request {
            Request::All => self.load_version().await.filter(|v| !v.is_empty()),
            Request::Names(_) => None,
        };
        let data = CatalogData {
            instance: self.v1.http.base_url.clone(),
            version,
            captured_at: iso_date((self.clock.now_ms)()),
            resources,
        };
        let source = if loaded.iter().any(|l| l.from_fallback) {
            Source::Snapshot
        } else {
            Source::Live
        };
        let names = entries.iter().map(|e| e.name.clone()).collect();
        Arc::new(Catalog::new(data, source, Some(names)))
    }

    async fn load_index(&self) -> Option<Arc<Vec<IndexEntry>>> {
        self.index
            .run(
                || async {
                    match loader::load_index(&self.v1).await {
                        Ok(entries) => Some(Arc::new(entries)),
                        Err(e) => {
                            (self.log)(&format!("catalog: Index/meta failed ({})", e.message));
                            None
                        }
                    }
                },
                Option::is_some,
            )
            .await
    }

    async fn load_version(&self) -> Option<String> {
        self.version
            .run(|| loader::load_version(&self.v1), Option::is_some)
            .await
    }

    async fn load_resource(&self, t: &Target) -> LoadedResource {
        let key = t.path.to_lowercase();
        let flight = self
            .loads
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        flight
            .run(
                || loader::load_resource(&self.v1, t, &self.log, Some(snapshot)),
                |l| {
                    if l.is_final
                        && let Some(r) = &l.resource
                    {
                        self.known.lock().unwrap().insert(key, r.clone());
                    }
                    l.is_final
                },
            )
            .await
    }
}

/// What to load for a name: its index entry (by name or path), or an unlisted
/// history (`UserStoryHistory`, `UserStorySimpleHistories`) or `GeneralConversions`.
/// None when the instance has no such resource.
fn target(name: &str, entries: &[IndexEntry]) -> Option<Target> {
    static HISTORY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(.+?)(simple)?histor(?:y|ies)$").unwrap());
    static CONVERSIONS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^generalconversions?$").unwrap());
    let n = js_trim(name);
    if let Some(entry) = entries
        .iter()
        .find(|e| same_name(&e.name, n) || same_name(&e.path, n))
    {
        return Some(Target::listed(entry));
    }
    if let Some(history) = HISTORY.captures(n)
        && let Some(base) = entries.iter().find(|e| same_name(&e.name, &history[1]))
    {
        let simple = if history.get(2).is_some() {
            "Simple"
        } else {
            ""
        };
        return Some(Target::unlisted(format!("{}{simple}Histories", base.name)));
    }
    CONVERSIONS
        .is_match(n)
        .then(|| Target::unlisted("GeneralConversions".to_string()))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;

    use serde_json::json;

    use super::loader::{load_catalog, parse_index, parse_meta_xml};
    use super::*;
    use crate::http::{BoxFuture, Http, Request as HttpRequest, Response, Transport};

    pub const INDEX: &str = include_str!("../../tests/fixtures/index-meta.json");
    pub const CONTEXT_META: &str = include_str!("../../tests/fixtures/context-meta.xml");
    pub const STORY_META: &str = include_str!("../../tests/fixtures/userstory-meta.json");

    type Reply = Box<dyn Fn() -> (u16, String) + Send + Sync>;

    enum Pattern {
        /// Compared case-insensitively.
        Exact(&'static str),
        Regex(Regex),
    }

    impl Pattern {
        fn matches(&self, path: &str) -> bool {
            match self {
                Pattern::Exact(p) => p.eq_ignore_ascii_case(path),
                Pattern::Regex(re) => re.is_match(path),
            }
        }
    }

    #[derive(Default)]
    struct State {
        routes: Mutex<Vec<(Pattern, Reply)>>,
        calls: Mutex<Vec<String>>,
        unmatched: Mutex<Vec<String>>,
    }

    /// GET routes answering like Targetprocess; unmatched requests answer 599 and are recorded.
    /// Every answer waits one turn of the runtime, so concurrent callers really overlap.
    #[derive(Clone, Default)]
    pub struct Stub(Arc<State>);

    impl Stub {
        fn route(self, pattern: Pattern, reply: Reply, first: bool) -> Self {
            let mut routes = self.0.routes.lock().unwrap();
            if first {
                routes.insert(0, (pattern, reply));
            } else {
                routes.push((pattern, reply));
            }
            drop(routes);
            self
        }

        pub fn get(self, path: &'static str, status: u16, body: impl Into<String>) -> Self {
            let body = body.into();
            self.route(
                Pattern::Exact(path),
                Box::new(move || (status, body.clone())),
                false,
            )
        }

        pub fn get_re(self, re: &str, status: u16, body: impl Into<String>) -> Self {
            let body = body.into();
            let pattern = Pattern::Regex(Regex::new(re).unwrap());
            self.route(pattern, Box::new(move || (status, body.clone())), false)
        }

        pub fn get_with(
            self,
            path: &'static str,
            reply: impl Fn() -> (u16, String) + Send + Sync + 'static,
        ) -> Self {
            self.route(Pattern::Exact(path), Box::new(reply), false)
        }

        /// A route that takes precedence over every route registered so far.
        pub fn first(
            self,
            path: &'static str,
            reply: impl Fn() -> (u16, String) + Send + Sync + 'static,
        ) -> Self {
            self.route(Pattern::Exact(path), Box::new(reply), true)
        }

        pub fn calls(&self) -> Vec<String> {
            self.0.calls.lock().unwrap().clone()
        }

        pub fn count(&self, path: &str) -> usize {
            self.calls()
                .iter()
                .filter(|c| c.eq_ignore_ascii_case(path))
                .count()
        }

        pub fn unmatched(&self) -> Vec<String> {
            self.0.unmatched.lock().unwrap().clone()
        }

        pub fn v1(&self) -> V1 {
            let http = Http::new(
                "https://tp.example".into(),
                "t0ken".into(),
                Box::new(self.clone()),
                Arc::new(|_| {}),
            );
            V1::new(Arc::new(http))
        }
    }

    impl Transport for Stub {
        fn send(
            &self,
            request: HttpRequest,
            _timeout: Duration,
        ) -> BoxFuture<'_, Result<Response, String>> {
            Box::pin(async move {
                tokio::task::yield_now().await;
                let url = url::Url::parse(&request.url).unwrap();
                let path = url.path().to_string();
                self.0.calls.lock().unwrap().push(path.clone());
                let routes = self.0.routes.lock().unwrap();
                let (status, text) = match routes.iter().find(|(p, _)| p.matches(&path)) {
                    Some((_, reply)) => reply(),
                    None => {
                        self.0.unmatched.lock().unwrap().push(path.clone());
                        (599, format!("UNSTUBBED GET {path}"))
                    }
                };
                Ok(Response {
                    status,
                    location: None,
                    body: Ok(text),
                })
            })
        }
    }

    fn not_found() -> String {
        json!({ "Status": "NotFound", "Message": "Not Found" }).to_string()
    }

    fn unavailable() -> (u16, String) {
        (503, json!({ "Message": "Service Unavailable" }).to_string())
    }

    /// An instance with five listed resources (SickLeave broken) and two unlisted ones.
    pub fn instance() -> Stub {
        Stub::default()
            .get("/api/v1/Index/meta", 200, INDEX)
            .get("/api/v1/UserStories/meta", 200, STORY_META)
            .get("/api/v1/Context/meta", 200, CONTEXT_META)
            .get("/api/v1/SickLeaves/meta", 404, not_found())
            .get(
                "/api/v1/AcceptanceCriterions/meta",
                200,
                json!({ "Name": "AcceptanceCriterion", "Uri": "https://tp.example/api/v1/AcceptanceCriterions", "CanCreate": true, "CanUpdate": true, "CanDelete": true }).to_string(),
            )
            .get(
                "/api/v1/ActionItems/meta",
                200,
                json!({ "Name": "ActionItem", "Uri": "https://tp.example/api/v1/ActionItems", "CanCreate": true, "CanUpdate": true, "CanDelete": true }).to_string(),
            )
            .get(
                "/api/v1/UserStoryHistories/meta",
                200,
                json!({ "Name": "UserStoryHistory", "Uri": "https://tp.example/api/v1/UserStoryHistories", "CanCreate": false, "CanUpdate": false, "CanDelete": false }).to_string(),
            )
            .get(
                "/api/v1/GeneralConversions/meta",
                200,
                json!({ "Name": "GeneralConversion", "Uri": "https://tp.example/api/v1/GeneralConversions" }).to_string(),
            )
            .get("/api/v1/Context", 200, json!({ "Version": "2609.2.0.3492" }).to_string())
            .get_re(r"Histories/meta$", 404, not_found())
    }

    fn names(list: &[&str]) -> Request {
        Request::Names(list.iter().map(|s| s.to_string()).collect())
    }

    fn quiet() -> Logger {
        Arc::new(|_| {})
    }

    fn provider(stub: &Stub) -> Provider {
        Provider::live(stub.v1(), quiet(), Clock::fixed(0))
    }

    #[test]
    fn parse_index_reads_every_entry_despite_the_repeated_json_key() {
        let parsed: Value = serde_json::from_str(INDEX).unwrap();
        // What a JSON parser sees: IndexUri + one entry.
        assert_eq!(parsed.as_object().unwrap().len(), 2);
        let entries: Vec<(String, String)> = parse_index(INDEX)
            .into_iter()
            .map(|e| (e.name, e.path))
            .collect();
        let want = [
            ("AcceptanceCriterion", "AcceptanceCriterions"),
            ("ActionItem", "ActionItems"),
            ("Context", "Context"),
            ("SickLeave", "SickLeaves"),
            ("UserStory", "UserStories"),
        ];
        assert_eq!(
            entries,
            want.map(|(n, p)| (n.to_string(), p.to_string())).to_vec()
        );
    }

    #[test]
    fn parse_meta_xml_converts_xml_only_metadata() {
        let meta = parse_meta_xml(CONTEXT_META).unwrap();
        assert_eq!(meta["Name"], "Context");
        assert_eq!(meta["CanCreate"], true);
        let collections = meta["ResourceMetadataPropertiesDescription"]
            ["ResourceMetadataPropertiesResourceCollectionsDescription"]["Items"]
            .as_array()
            .unwrap();
        let names: Vec<&str> = collections
            .iter()
            .map(|c| c["Name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "SelectedProjects",
                "SelectedTeams",
                "Processes",
                "GlobalTerms",
                "CustomFields"
            ]
        );
        assert_eq!(collections[0]["CanAdd"], false);
        assert_eq!(collections[0]["CanRemove"], false);
        assert_eq!(collections[0]["CanSet"], false);
    }

    #[tokio::test]
    async fn load_catalog_marks_broken_resources_unavailable_and_probes_unlisted_ones() {
        let stub = instance();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-23T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let data = load_catalog(&stub.v1(), &quiet(), now, None).await.unwrap();
        let by_name = |n: &str| data.resources.iter().find(|r| r.name == n).unwrap().clone();
        assert_eq!(data.version.as_deref(), Some("2609.2.0.3492"));
        assert_eq!(data.captured_at, "2026-09-23T00:00:00.000Z");

        let sick = by_name("SickLeave");
        assert!(sick.listed && !sick.available);
        let history = by_name("UserStoryHistory");
        assert!(!history.listed && history.available);
        assert_eq!(history.path, "UserStoryHistories");
        let conversion = by_name("GeneralConversion");
        assert!(!conversion.listed);
        assert_eq!(conversion.path, "GeneralConversions");
        assert_eq!(by_name("Context").collections.len(), 5);

        let story = by_name("UserStory");
        assert_eq!(story.path, "UserStories");
        assert!(story.can_create);
        assert_eq!(story.bases, ["Assignable"]);
        let name = story.values.iter().find(|f| f.name == "Name").unwrap();
        assert!(name.required && name.can_set);
        let comments = story
            .collections
            .iter()
            .find(|c| c.name == "Comments")
            .unwrap();
        assert!(comments.can_add && comments.can_remove);

        for name in ["SickLeave", "UserStory", "Context"] {
            assert_eq!(stub.count(&format!("/api/v1/{name}Histories/meta")), 1);
            assert_eq!(
                stub.count(&format!("/api/v1/{name}SimpleHistories/meta")),
                1
            );
        }
        assert_eq!(stub.unmatched(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn load_catalog_fails_when_the_index_itself_is_unavailable() {
        let stub = Stub::default().get(
            "/api/v1/Index/meta",
            401,
            json!({ "Message": "Unauthorized" }).to_string(),
        );
        let e = load_catalog(&stub.v1(), &quiet(), 0, None)
            .await
            .unwrap_err();
        assert_eq!(e.status, 401);
    }

    #[tokio::test]
    async fn load_catalog_retries_a_failed_meta_once_then_uses_the_snapshot_entry() {
        let story_calls = Arc::new(AtomicUsize::new(0));
        let counter = story_calls.clone();
        let stub = Stub::default()
            .get("/api/v1/Index/meta", 200, INDEX)
            .get_with("/api/v1/UserStories/meta", move || {
                counter.fetch_add(1, AtomicOrdering::SeqCst);
                unavailable()
            })
            .get(
                "/api/v1/Context",
                200,
                json!({ "Version": "x" }).to_string(),
            )
            .get_re(r"/meta$", 404, not_found());
        let data = load_catalog(&stub.v1(), &quiet(), 0, Some(snapshot))
            .await
            .unwrap();
        assert_eq!(story_calls.load(AtomicOrdering::SeqCst), 2);
        let story = data
            .resources
            .iter()
            .find(|r| r.name == "UserStory")
            .unwrap();
        assert!(story.available && story.listed);
        assert_eq!(story.path, "UserStories");
        assert!(story.collections.len() > 10);
    }

    #[tokio::test]
    async fn load_catalog_a_404_is_final_no_retry_and_no_fallback() {
        let stub = Stub::default()
            .get("/api/v1/Index/meta", 200, INDEX)
            .get(
                "/api/v1/Context",
                200,
                json!({ "Version": "x" }).to_string(),
            )
            .get_re(r"/meta$", 404, not_found());
        let data = load_catalog(&stub.v1(), &quiet(), 0, Some(snapshot))
            .await
            .unwrap();
        assert_eq!(stub.count("/api/v1/SickLeaves/meta"), 1);
        assert!(
            !data
                .resources
                .iter()
                .find(|r| r.name == "SickLeave")
                .unwrap()
                .available
        );
    }

    #[tokio::test]
    async fn provider_loads_only_the_requested_resources_once_for_concurrent_callers() {
        let stub = instance();
        let provider = provider(&stub);
        let (story, stories) = (names(&["UserStory"]), names(&["userstories"]));
        let (a, b) = tokio::join!(provider.get(&story), provider.get(&stories));
        for catalog in [&a, &b] {
            assert_eq!(catalog.source, Source::Live);
            let story = catalog.find("UserStory").unwrap();
            assert_eq!(story.path, "UserStories");
            assert!(story.available && story.listed);
        }
        provider.get(&names(&["UserStory"])).await;
        assert_eq!(
            stub.calls(),
            ["/api/v1/Index/meta", "/api/v1/UserStories/meta"]
        );
        // Unknown names are checked against the index, which lists every resource.
        let unknown = a.resource("UserStorie").unwrap_err();
        assert!(
            unknown.starts_with("Unknown resource \"UserStorie\". Did you mean: UserStory"),
            "{unknown}"
        );
    }

    #[tokio::test]
    async fn provider_resolves_unlisted_history_resources_on_demand_and_a_404_is_final() {
        let stub = instance();
        let provider = provider(&stub);
        let catalog = provider
            .get(&names(&[
                "UserStoryHistories",
                "UserStorySimpleHistory",
                "SickLeave",
                "GeneralConversions",
            ]))
            .await;
        let history = catalog.find("UserStoryHistory").unwrap();
        assert_eq!(history.path, "UserStoryHistories");
        assert!(!history.listed && history.available);
        assert!(catalog.find("UserStorySimpleHistories").is_none());
        let sick = catalog.find("SickLeave").unwrap();
        assert!(sick.listed && !sick.available);
        assert!(!catalog.find("GeneralConversions").unwrap().listed);
        provider
            .get(&names(&["UserStorySimpleHistories", "SickLeaves"]))
            .await;
        assert_eq!(stub.count("/api/v1/UserStorySimpleHistories/meta"), 1);
        assert_eq!(stub.count("/api/v1/SickLeaves/meta"), 1);
    }

    #[tokio::test]
    async fn provider_serves_the_snapshot_entry_for_a_failed_meta_and_retries_on_the_next_call() {
        let story_meta = Arc::new(AtomicUsize::new(0));
        let counter = story_meta.clone();
        let stub = instance().first("/api/v1/UserStories/meta", move || {
            if counter.fetch_add(1, AtomicOrdering::SeqCst) < 2 {
                unavailable()
            } else {
                (200, STORY_META.to_string())
            }
        });
        let provider = provider(&stub);
        let first = provider.get(&names(&["UserStory"])).await;
        assert_eq!(first.source, Source::Snapshot);
        assert!(first.find("UserStory").unwrap().collections.len() > 10);
        let second = provider.get(&names(&["UserStory"])).await;
        assert_eq!(second.source, Source::Live);
        assert_eq!(story_meta.load(AtomicOrdering::SeqCst), 3);
    }

    #[tokio::test]
    async fn provider_falls_back_to_the_snapshot_when_the_index_fails_and_retries_on_the_next_call()
    {
        let index = Arc::new(AtomicUsize::new(0));
        let counter = index.clone();
        let stub = instance().first("/api/v1/Index/meta", move || {
            if counter.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                (503, json!({ "Message": "down" }).to_string())
            } else {
                (200, INDEX.to_string())
            }
        });
        let provider = provider(&stub);
        assert_eq!(
            provider.get(&names(&["UserStory"])).await.source,
            Source::Snapshot
        );
        assert_eq!(
            provider.get(&names(&["UserStory"])).await.source,
            Source::Live
        );
        assert_eq!(index.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn provider_all_loads_every_listed_resource_the_unlisted_ones_and_the_version() {
        let stub = instance();
        let catalog = provider(&stub).get(&Request::All).await;
        assert_eq!(catalog.source, Source::Live);
        assert_eq!(catalog.data.version.as_deref(), Some("2609.2.0.3492"));
        let mut got: Vec<&str> = catalog
            .resources()
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                "AcceptanceCriterion",
                "ActionItem",
                "Context",
                "GeneralConversion",
                "SickLeave",
                "UserStory",
                "UserStoryHistory"
            ]
        );
        assert_eq!(stub.unmatched(), Vec::<String>::new());
    }

    #[test]
    fn catalog_finds_by_name_or_path_case_insensitively() {
        let catalog = snapshot();
        assert_eq!(catalog.find("userstory").unwrap().path, "UserStories");
        assert_eq!(catalog.find("UserStories").unwrap().name, "UserStory");
    }

    #[test]
    fn catalog_suggests_close_names_for_unknown_resources() {
        let message = snapshot().resource("UserStorys").unwrap_err();
        assert!(
            Regex::new(r"Did you mean: .*UserStory")
                .unwrap()
                .is_match(&message),
            "{message}"
        );
    }

    #[test]
    fn catalog_refuses_unavailable_resources() {
        let message = snapshot().resource("SickLeave").unwrap_err();
        assert!(message.contains("metadata is unavailable"), "{message}");
    }

    #[test]
    fn suggestions_ranks_prefixes_first() {
        assert_eq!(
            suggestions("Bug", &["Bug", "BugHistory", "Build", "Epic"], 8)[0],
            "Bug"
        );
    }

    #[test]
    fn snapshot_matches_the_researched_baseline_of_our_instance() {
        let catalog = snapshot();
        let listed: Vec<_> = catalog.resources().iter().filter(|r| r.listed).collect();
        let collections: Vec<_> = catalog
            .resources()
            .iter()
            .flat_map(|r| &r.collections)
            .collect();
        assert_eq!(listed.len(), 89);
        let unavailable: Vec<&str> = listed
            .iter()
            .filter(|r| !r.available)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(unavailable, ["SickLeave"]);
        assert_eq!(collections.len(), 1054);
        assert_eq!(
            collections
                .iter()
                .filter(|c| c.can_add || c.can_remove)
                .count(),
            543
        );
    }

    #[test]
    fn snapshot_includes_the_unlisted_history_and_conversion_resources() {
        let catalog = snapshot();
        let unlisted: Vec<&str> = catalog
            .resources()
            .iter()
            .filter(|r| !r.listed)
            .map(|r| r.path.as_str())
            .collect();
        for path in [
            "GeneralConversions",
            "TaskHistories",
            "UserStorySimpleHistories",
        ] {
            assert!(unlisted.contains(&path), "{path}");
        }
    }

    #[test]
    fn snapshot_has_unique_names_and_paths() {
        let mut owner: HashMap<String, &str> = HashMap::new();
        for r in snapshot().resources() {
            let mut keys = vec![r.name.to_lowercase(), r.path.to_lowercase()];
            keys.dedup();
            for key in keys {
                assert_eq!(*owner.get(&key).unwrap_or(&r.name.as_str()), r.name);
                owner.insert(key, &r.name);
            }
        }
    }

    #[test]
    fn snapshot_round_trips_through_the_typed_catalog() {
        let raw: Value = serde_json::from_str(include_str!("snapshot.json")).unwrap();
        assert_eq!(CatalogData::from_json(&raw).to_json(), raw);
    }

    #[test]
    fn locale_compare_ignores_case_then_puts_lower_case_first() {
        assert_eq!(locale_compare("bug", "Build"), Ordering::Less);
        assert_eq!(
            locale_compare("UserStory", "UserStoryHistory"),
            Ordering::Less
        );
        assert_eq!(locale_compare("a", "A"), Ordering::Less);
        assert_eq!(locale_compare("Epic", "epic"), Ordering::Greater);
    }
}
