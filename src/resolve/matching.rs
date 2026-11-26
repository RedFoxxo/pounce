//! Name or id → one entry of a list, never guessing (port of `match.ts`).

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::http::TpErr;

/// One entry offered when a name is ambiguous or unknown.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub name: String,
    /// Extra fields shown after `id` and `name` (e.g. `login`).
    pub extra: Map<String, Value>,
}

impl Candidate {
    /// `{id, name, ...extra}`.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("name".into(), json!(self.name));
        out.extend(self.extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Nothing matched.
    NotFound,
    /// Several matched.
    Ambiguous,
    /// The lookup itself failed.
    Error,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NotFound => "none",
            Reason::Ambiguous => "ambiguous",
            Reason::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    /// One-line reason, e.g. `No role matches "Dev".`
    pub message: String,
    pub reason: Reason,
    /// Every candidate for an ambiguous name, or close suggestions for an unknown one.
    pub candidates: Option<Vec<Candidate>>,
    /// Set when the lookup itself failed at the HTTP level.
    pub error: Option<TpErr>,
}

impl Unresolved {
    fn new(reason: Reason, message: String) -> Box<Self> {
        Box::new(Unresolved {
            message,
            reason,
            candidates: None,
            error: None,
        })
    }

    /// A failed load of the list to match against.
    pub fn failed(message: impl Into<String>, error: TpErr) -> Box<Self> {
        Box::new(Unresolved {
            message: message.into(),
            reason: Reason::Error,
            candidates: None,
            error: Some(error),
        })
    }
}

/// Boxed: the failure carries a message, candidates and the HTTP error.
pub type Resolved<T> = Result<T, Box<Unresolved>>;

/// Extra fields shown for a candidate.
pub type Describe<'f, T> = &'f dyn Fn(&T) -> Map<String, Value>;

/// How to read the entries being matched.
pub struct MatchSpec<'f, T> {
    /// What is being resolved, e.g. `user`, `role`.
    pub kind: &'f str,
    pub id: &'f dyn Fn(&T) -> i64,
    /// Display name.
    pub name: &'f dyn Fn(&T) -> String,
    /// Every string the input may equal, e.g. full name, login, email; `None` for a missing one.
    pub keys: &'f dyn Fn(&T) -> Vec<Option<String>>,
    /// Extra fields shown for candidates.
    pub describe: Option<Describe<'f, T>>,
}

/// Case-, accent- and whitespace-insensitive form used for every name comparison.
pub fn norm(s: &str) -> String {
    let stripped: String = s
        .nfkd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect();
    let lower = js_trim(&stripped).to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut space = false;
    for c in lower.chars() {
        if is_js_space(c) {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

/// JavaScript's `\s`, so trimming and splitting match the TypeScript build.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `Number(raw) === id` for an all-digit input.
fn same_id(raw: &str, id: i64) -> bool {
    raw.parse::<f64>().is_ok_and(|n| n == id as f64)
}

fn candidate<T>(spec: &MatchSpec<T>, item: &T) -> Candidate {
    Candidate {
        id: (spec.id)(item),
        name: (spec.name)(item),
        extra: spec.describe.map(|d| d(item)).unwrap_or_default(),
    }
}

/// Each item's keys, normalized once; `None` where the key is missing or empty.
fn normalized_keys<T>(spec: &MatchSpec<T>, items: &[&T]) -> Vec<Vec<Option<String>>> {
    items
        .iter()
        .map(|item| {
            (spec.keys)(item)
                .into_iter()
                .map(|k| k.filter(|k| !k.is_empty()).map(|k| norm(&k)))
                .collect()
        })
        .collect()
}

fn listed(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|c| format!("{} ({})", c.name, c.id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolves a name or numeric id. Never guesses:
/// - numeric input matches the id exactly;
/// - otherwise one exact (case/accent-insensitive) match on any key wins;
/// - otherwise one partial match (every word of the input is found) wins;
/// - several matches at the same level → error listing every candidate;
/// - no match → error with close suggestions.
pub fn r#match<'a, T>(input: &str, spec: &MatchSpec<T>, items: &[&'a T]) -> Resolved<&'a T> {
    let raw = js_trim(input);
    if is_digits(raw) {
        return match items.iter().find(|item| same_id(raw, (spec.id)(item))) {
            Some(found) => Ok(found),
            None => Err(Unresolved::new(
                Reason::NotFound,
                format!("No {} has id {raw}.", spec.kind),
            )),
        };
    }
    let q = norm(raw);
    if q.is_empty() {
        return Err(Unresolved::new(
            Reason::NotFound,
            format!("Empty {} name.", spec.kind),
        ));
    }
    let keys = normalized_keys(spec, items);

    // Exact matches, by key priority: a hit on the first key (the name) beats a hit on a later one (an abbreviation).
    let exact = exact_by_priority(&q, &keys, items);
    match exact.as_slice() {
        [one] => return Ok(one),
        [] => {}
        _ => return Err(ambiguous(spec, raw, &exact)),
    }

    let words: Vec<&str> = q.split(' ').collect();
    let partial: Vec<&'a T> = items
        .iter()
        .zip(&keys)
        .filter(|(_, ks)| {
            ks.iter()
                .flatten()
                .any(|k| words.iter().all(|w| k.contains(w)))
        })
        .map(|(item, _)| *item)
        .collect();
    match partial.as_slice() {
        [one] => return Ok(one),
        [] => {}
        _ => return Err(ambiguous(spec, raw, &partial)),
    }

    let q16: Vec<u16> = q.encode_utf16().collect();
    let threshold = 2.max(q16.len() / 3);
    let mut scored: Vec<(usize, &T)> = items
        .iter()
        .zip(&keys)
        .filter_map(|(item, ks)| {
            let score = ks.iter().flatten().map(|k| distance(&q16, k)).min()?;
            (score <= threshold).then_some((score, *item))
        })
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    let close: Vec<Candidate> = scored
        .iter()
        .take(5)
        .map(|(_, item)| candidate(spec, item))
        .collect();
    let mut message = format!("No {} matches \"{raw}\".", spec.kind);
    if !close.is_empty() {
        message.push_str(&format!(" Did you mean: {}?", listed(&close)));
    }
    let mut out = Unresolved::new(Reason::NotFound, message);
    if !close.is_empty() {
        out.candidates = Some(close);
    }
    Err(out)
}

fn exact_by_priority<'a, T>(q: &str, keys: &[Vec<Option<String>>], items: &[&'a T]) -> Vec<&'a T> {
    let width = keys.iter().map(Vec::len).max().unwrap_or(0);
    for k in 0..width {
        let hits: Vec<&'a T> = items
            .iter()
            .zip(keys)
            .filter(|(_, ks)| ks.get(k).and_then(Option::as_deref) == Some(q))
            .map(|(item, _)| *item)
            .collect();
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

fn ambiguous<T>(spec: &MatchSpec<T>, raw: &str, items: &[&T]) -> Box<Unresolved> {
    let candidates: Vec<Candidate> = items.iter().map(|item| candidate(spec, item)).collect();
    Box::new(Unresolved {
        message: format!(
            "\"{raw}\" matches {} {}s; pass one id: {}.",
            items.len(),
            spec.kind,
            listed(&candidates)
        ),
        reason: Reason::Ambiguous,
        candidates: Some(candidates),
        error: None,
    })
}

/// Edit distance in UTF-16 units, as the TypeScript build counts.
fn distance(a: &[u16], b: &str) -> usize {
    let b: Vec<u16> = b.encode_utf16().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
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

fn inactive<T>(spec: &MatchSpec<T>, item: &T) -> Box<Unresolved> {
    let mut kind = spec.kind.chars();
    let kind = match kind.next() {
        Some(first) => first.to_uppercase().chain(kind).collect::<String>(),
        None => String::new(),
    };
    Unresolved::new(
        Reason::NotFound,
        format!(
            "{kind} {} ({}) is inactive or deleted.",
            (spec.name)(item),
            (spec.id)(item)
        ),
    )
}

/// Like `match`, but for directories with inactive entries. An exact name (or
/// id) is looked up across everyone first, so an exact hit on an inactive entry
/// is reported as inactive instead of silently resolving to an active entry
/// that merely contains the name. Partial matches consider active entries only.
pub fn match_active<'a, T>(
    input: &str,
    spec: &MatchSpec<T>,
    items: &[&'a T],
    is_active: &dyn Fn(&T) -> bool,
) -> Resolved<&'a T> {
    let raw = js_trim(input);
    let exact: Vec<&'a T> = if is_digits(raw) {
        items
            .iter()
            .copied()
            .filter(|item| same_id(raw, (spec.id)(item)))
            .collect()
    } else {
        exact_by_priority(&norm(raw), &normalized_keys(spec, items), items)
    };
    if let Some(first) = exact.first() {
        let active: Vec<&'a T> = exact
            .iter()
            .copied()
            .filter(|item| is_active(item))
            .collect();
        return match active.as_slice() {
            [one] => Ok(one),
            [] => Err(inactive(spec, first)),
            _ => Err(ambiguous(spec, raw, &active)),
        };
    }
    let (active, gone): (Vec<&'a T>, Vec<&'a T>) =
        items.iter().copied().partition(|item| is_active(item));
    let found = r#match(raw, spec, &active);
    match &found {
        Err(e) if e.reason == Reason::NotFound => match r#match(raw, spec, &gone) {
            Ok(item) => Err(inactive(spec, item)),
            Err(_) => found,
        },
        _ => found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_folds_case_accents_and_spaces() {
        assert_eq!(norm("  Ńiccolò \t Rossì "), "niccolo rossi");
    }
}
