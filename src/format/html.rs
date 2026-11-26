//! HTML <-> text for descriptions and comments: Targetprocess stores rich text as
//! HTML (or Markdown behind a `<!--markdown-->` marker); AI clients read light Markdown.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// JavaScript's `\s` (what `trim` removes), so the regexes and trims match the TypeScript build.
const JS_SPACE: &str = r"[\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";

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

/// Latin-1 named entities (U+00A0–U+00FF), in code point order.
const LATIN1: &str = "nbsp iexcl cent pound curren yen brvbar sect uml copy ordf laquo not shy reg macr deg plusmn sup2 sup3 acute micro para middot cedil sup1 ordm raquo frac14 frac12 frac34 iquest Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml";

/// Named entities, exact case (`&Ograve;` and `&ograve;` differ).
static NAMED: LazyLock<HashMap<&'static str, char>> = LazyLock::new(|| {
    let mut map: HashMap<&'static str, char> = LATIN1.split(' ').zip('\u{a0}'..='\u{ff}').collect();
    map.extend([
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("OElig", 'Œ'),
        ("oelig", 'œ'),
        ("Scaron", 'Š'),
        ("scaron", 'š'),
        ("Yuml", 'Ÿ'),
        ("fnof", 'ƒ'),
        ("circ", 'ˆ'),
        ("tilde", '˜'),
        ("ensp", '\u{2002}'),
        ("emsp", '\u{2003}'),
        ("thinsp", '\u{2009}'),
        ("zwnj", '\u{200c}'),
        ("zwj", '\u{200d}'),
        ("ndash", '–'),
        ("mdash", '—'),
        ("lsquo", '‘'),
        ("rsquo", '’'),
        ("sbquo", '‚'),
        ("ldquo", '“'),
        ("rdquo", '”'),
        ("bdquo", '„'),
        ("dagger", '†'),
        ("Dagger", '‡'),
        ("bull", '•'),
        ("hellip", '…'),
        ("permil", '‰'),
        ("lsaquo", '‹'),
        ("rsaquo", '›'),
        ("euro", '€'),
        ("trade", '™'),
        ("larr", '←'),
        ("uarr", '↑'),
        ("rarr", '→'),
        ("darr", '↓'),
        ("harr", '↔'),
        ("check", '✓'),
    ]);
    map
});

static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[A-Za-z][A-Za-z0-9]*);").unwrap());

/// A numeric character reference's code point, if it names one (as `String.fromCodePoint` accepts it).
fn code_point(code: &str) -> Option<char> {
    let (digits, radix) = match code.as_bytes().get(1) {
        Some(b'x' | b'X') => (&code[2..], 16),
        _ => (&code[1..], 10),
    };
    // Overflow means beyond 0x10FFFF, which is refused anyway.
    let n = u32::from_str_radix(digits, radix).ok()?;
    if n == 0 || n > 0x10ffff {
        return None;
    }
    // A lone surrogate is valid in a JS string but not in Rust: shown as U+FFFD.
    Some(char::from_u32(n).unwrap_or('\u{fffd}'))
}

pub fn decode_entities(text: &str) -> String {
    ENTITY
        .replace_all(text, |c: &Captures| {
            let code = &c[1];
            let decoded = if code.starts_with('#') {
                code_point(code)
            } else {
                NAMED.get(code).copied()
            };
            match decoded {
                Some(ch) => ch.to_string(),
                None => c[0].to_string(),
            }
        })
        .into_owned()
}

static MARKDOWN_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^{JS_SPACE}*<!--{JS_SPACE}*markdown{JS_SPACE}*-->{JS_SPACE}*"
    ))
    .unwrap()
});

// ---------------------------------------------------------------- HTML → light Markdown

struct Element {
    tag: String,
    attrs: HashMap<String, String>,
    children: Vec<Node>,
}

enum Node {
    Text(String),
    El(Element),
}

const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "col", "wbr", "area", "base", "source",
];
const SKIP: &[&str] = &["script", "style", "head", "title"];
/// Opening one of these closes an open element of the same kind (lenient HTML).
const SELF_CLOSING_SIBLINGS: &[&str] = &["li", "p", "tr", "td", "th", "dt", "dd"];
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "address",
    "center",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "hr",
];

static ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"([A-Za-z_:][0-9A-Za-z_:.-]*){JS_SPACE}*(?:={JS_SPACE}*("([^"]*)"|'([^']*)'|([^\t\n\x0B\x0C\r \x{{a0}}\x{{1680}}\x{{2000}}-\x{{200a}}\x{{2028}}\x{{2029}}\x{{202f}}\x{{205f}}\x{{3000}}\x{{feff}}"'=<>`]+)))?"#
    ))
    .unwrap()
});
// Linear on malformed input: a tag cannot contain an unquoted "<", and an unclosed comment runs to the end (as in browsers).
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<!--[\s\S]*?(?:-->|$)|</?([A-Za-z][A-Za-z0-9]*)(?-u:\b)((?:[^<>"']|"[^"]*"|'[^']*')*)>|[^<]+|<"#).unwrap()
});
static SELF_CLOSED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"/{JS_SPACE}*>$")).unwrap());

fn parse_attrs(raw: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    for m in ATTR.captures_iter(raw) {
        let value = m
            .get(3)
            .or(m.get(4))
            .or(m.get(5))
            .map_or("", |v| v.as_str());
        attrs.insert(m[1].to_ascii_lowercase(), decode_entities(value));
    }
    attrs
}

/// Closes the innermost open element into its parent.
fn close_top(stack: &mut Vec<Element>) {
    if stack.len() < 2 {
        return;
    }
    if let Some(el) = stack.pop()
        && let Some(parent) = stack.last_mut()
    {
        parent.children.push(Node::El(el));
    }
}

fn parse_html(html: &str) -> Vec<Node> {
    let mut stack = vec![Element {
        tag: "#root".into(),
        attrs: HashMap::new(),
        children: Vec::new(),
    }];
    let mut skipping: Option<String> = None;
    for m in TOKEN.captures_iter(html) {
        let token = &m[0];
        let tag_name = m.get(1).map(|t| t.as_str().to_ascii_lowercase());
        if let Some(skip) = &skipping {
            if tag_name.as_ref() == Some(skip) && token.starts_with("</") {
                skipping = None;
            }
            continue;
        }
        if token.starts_with("<!--") {
            continue;
        }
        let Some(tag_name) = tag_name else {
            if let Some(top) = stack.last_mut() {
                top.children.push(Node::Text(token.to_string()));
            }
            continue;
        };
        if token.starts_with("</") {
            if let Some(at) = stack.iter().rposition(|e| e.tag == tag_name)
                && at > 0
            {
                while stack.len() > at {
                    close_top(&mut stack);
                }
            }
            continue;
        }
        let self_closed = SELF_CLOSED.is_match(token);
        if SKIP.contains(&tag_name.as_str()) {
            if !self_closed {
                skipping = Some(tag_name);
            }
            continue;
        }
        if SELF_CLOSING_SIBLINGS.contains(&tag_name.as_str())
            && stack.last().is_some_and(|t| t.tag == tag_name)
        {
            close_top(&mut stack);
        }
        let attrs = parse_attrs(m.get(2).map_or("", |a| a.as_str()));
        let el = Element {
            tag: tag_name,
            attrs,
            children: Vec::new(),
        };
        if !VOID.contains(&el.tag.as_str()) && !self_closed {
            stack.push(el);
        } else if let Some(top) = stack.last_mut() {
            top.children.push(Node::El(el));
        }
    }
    while stack.len() > 1 {
        close_top(&mut stack);
    }
    stack.pop().map(|root| root.children).unwrap_or_default()
}

fn block_el(node: &Node) -> Option<&Element> {
    match node {
        Node::El(el) if BLOCK.contains(&el.tag.as_str()) => Some(el),
        _ => None,
    }
}

/// Raw text of a subtree, entities decoded, whitespace kept (for code).
fn raw_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&decode_entities(t)),
            Node::El(el) if el.tag == "br" => out.push('\n'),
            Node::El(el) => out.push_str(&raw_text(&el.children)),
        }
    }
    out
}

/// Wraps the non-space core of `text` in a Markdown mark, keeping surrounding spaces outside.
fn mark(text: String, left: &str, right: &str) -> String {
    let rest = text.trim_start_matches(is_js_space);
    let core = rest.trim_end_matches(is_js_space);
    if core.is_empty() {
        return text;
    }
    let lead = &text[..text.len() - rest.len()];
    let trail = &rest[core.len()..];
    format!("{lead}{left}{core}{right}{trail}")
}

static INLINE_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\n\r\x0C]+").unwrap());

fn inline(nodes: &[Node]) -> String {
    nodes.iter().map(inline_node).collect()
}

fn attr<'a>(el: &'a Element, name: &str) -> &'a str {
    el.attrs.get(name).map_or("", String::as_str)
}

fn inline_node(node: &Node) -> String {
    let el = match node {
        Node::Text(t) => {
            return INLINE_SPACE
                .replace_all(&decode_entities(t), " ")
                .into_owned();
        }
        Node::El(el) => el,
    };
    let inner = || inline(&el.children);
    match el.tag.as_str() {
        "br" => "\n".into(),
        "strong" | "b" => mark(inner(), "**", "**"),
        "em" | "i" => mark(inner(), "*", "*"),
        "s" | "del" | "strike" => mark(inner(), "~~", "~~"),
        "code" | "kbd" | "samp" => mark(raw_text(&el.children).replace('\n', " "), "`", "`"),
        "a" => {
            let href = attr(el, "href");
            let text = inner();
            let label = js_trim(&text);
            if href.is_empty() {
                label.to_string()
            } else if label.is_empty() || label == href {
                href.to_string()
            } else {
                format!("[{label}]({href})")
            }
        }
        "img" => {
            let src = attr(el, "src");
            let alt = attr(el, "alt");
            if !src.is_empty() {
                format!("![{alt}]({src})")
            } else if !alt.is_empty() {
                format!("[image: {alt}]")
            } else {
                "[image]".into()
            }
        }
        "input" => match (attr(el, "type"), el.attrs.contains_key("checked")) {
            ("checkbox", true) => "[x] ".into(),
            ("checkbox", false) => "[ ] ".into(),
            _ => String::new(),
        },
        // A block inside inline context (e.g. <p> in a <span>): keep it readable on its own lines.
        _ if block_el(node).is_some() => format!("\n{}\n", render_blocks(&el.children)),
        _ => inner(),
    }
}

#[derive(PartialEq)]
enum Kind {
    /// Targetprocess's one-div-per-line: joins with a single newline.
    Line,
    /// Joins with a blank line.
    Para,
}

struct Block {
    text: String,
    kind: Kind,
}

static LINE_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\x{a0}]+").unwrap());
static BLANK_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
static NEWLINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n+").unwrap());

fn clean_lines(text: &str) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|l| js_trim(&LINE_SPACE.replace_all(l, " ")).to_string())
        .collect();
    js_trim(&BLANK_LINES.replace_all(&lines.join("\n"), "\n\n")).to_string()
}

fn join_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Block> = None;
    for b in blocks {
        if b.kind == Kind::Para && b.text.is_empty() {
            continue;
        }
        if let Some(p) = prev {
            out.push_str(if p.kind == Kind::Line && b.kind == Kind::Line {
                "\n"
            } else {
                "\n\n"
            });
        }
        out.push_str(&b.text);
        prev = Some(b);
    }
    js_trim(&BLANK_LINES.replace_all(&out, "\n\n")).to_string()
}

fn indent(text: &str, by: &str) -> String {
    text.split('\n')
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{by}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Number.parseInt(text, 10)`: leading spaces, a sign, then digits; `None` for NaN.
fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim_start_matches(is_js_space);
    let (sign, digits) = match t.as_bytes().first() {
        Some(b'-') => (-1, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n = digits[..end].bytes().fold(0i64, |n, d| {
        n.saturating_mul(10).saturating_add(i64::from(d - b'0'))
    });
    Some(sign * n)
}

fn is_list(node: &Node) -> bool {
    matches!(node, Node::El(el) if el.tag == "ul" || el.tag == "ol")
}

fn render_list(tag: &str, start: Option<&str>, children: &[Node]) -> String {
    let ordered = tag == "ol";
    let mut n = match parse_int(start.unwrap_or("1")) {
        Some(0) | None => 1,
        Some(n) => n,
    };
    let mut items: Vec<String> = Vec::new();
    for child in children {
        let el = match child {
            Node::Text(t) => {
                if !js_trim(t).is_empty() {
                    items.push(format!("- {}", clean_lines(&inline_node(child))));
                }
                continue;
            }
            Node::El(el) => el,
        };
        if is_list(child) {
            items.push(indent(&render_el_list(el), "  "));
            continue;
        }
        let marker = if ordered {
            let m = format!("{n}.");
            n += 1;
            m
        } else {
            "-".to_string()
        };
        let is_li = el.tag == "li";
        let body = if is_li {
            let rest: Vec<&Node> = el.children.iter().filter(|c| !is_list(c)).collect();
            render_block_refs(&rest)
        } else {
            render_block_refs(&[child])
        };
        let pad = " ".repeat(marker.len() + 1);
        let mut lines = body.split('\n');
        let first = lines.next().unwrap_or("");
        let more: Vec<&str> = lines.collect();
        let mut item = format!("{marker} {first}")
            .trim_end_matches(is_js_space)
            .to_string();
        if !more.is_empty() {
            item.push('\n');
            item.push_str(&indent(&more.join("\n"), &pad));
        }
        if is_li {
            for sub in el.children.iter().filter(|c| is_list(c)) {
                if let Node::El(sub) = sub {
                    item.push('\n');
                    item.push_str(&indent(&render_el_list(sub), &pad));
                }
            }
        }
        items.push(item);
    }
    items.join("\n")
}

fn render_el_list(list: &Element) -> String {
    render_list(
        &list.tag,
        list.attrs.get("start").map(String::as_str),
        &list.children,
    )
}

fn render_table(table: &Element) -> String {
    fn collect<'a>(nodes: &'a [Node], rows: &mut Vec<&'a Element>) {
        for n in nodes {
            let Node::El(el) = n else { continue };
            match el.tag.as_str() {
                "tr" => rows.push(el),
                "thead" | "tbody" | "tfoot" => collect(&el.children, rows),
                _ => {}
            }
        }
    }
    let mut rows = Vec::new();
    collect(&table.children, &mut rows);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.children
                .iter()
                .filter_map(|c| match c {
                    Node::El(el) if el.tag == "td" || el.tag == "th" => Some(el),
                    _ => None,
                })
                .map(|c| {
                    NEWLINES
                        .replace_all(&clean_lines(&render_blocks(&c.children)), " ")
                        .replace('|', "\\|")
                })
                .collect()
        })
        .collect();
    let width = cells.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return String::new();
    }
    let line = |r: &[String]| {
        let padded: Vec<&str> = (0..width)
            .map(|i| r.get(i).map_or("", String::as_str))
            .collect();
        format!("| {} |", padded.join(" | "))
    };
    let mut out = vec![
        line(cells.first().map_or(&[][..], Vec::as_slice)),
        format!("|{}", " --- |".repeat(width)),
    ];
    out.extend(cells.iter().skip(1).map(|r| line(r)));
    out.join("\n")
}

static LANG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?:^|{JS_SPACE})(?:language|lang)-([0-9A-Za-z_+-]+)"
    ))
    .unwrap()
});
static TRAILING_NEWLINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n+$").unwrap());

fn block_of(node: &Node, el: &Element) -> Block {
    let para = |text: String| Block {
        text,
        kind: Kind::Para,
    };
    match el.tag.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let text = NEWLINES
                .replace_all(&clean_lines(&inline(&el.children)), " ")
                .into_owned();
            let level = usize::from(el.tag.as_bytes()[1] - b'0');
            para(if text.is_empty() {
                text
            } else {
                format!("{} {text}", "#".repeat(level))
            })
        }
        "hr" => para("---".into()),
        "pre" => {
            let code = match el.children.as_slice() {
                [Node::El(c)] if c.tag == "code" => c,
                _ => el,
            };
            let lang = LANG
                .captures(attr(code, "class"))
                .map_or("", |c| c.get(1).map_or("", |m| m.as_str()));
            let raw = raw_text(&code.children);
            let raw = raw.strip_prefix('\n').unwrap_or(&raw);
            para(format!(
                "```{lang}\n{}\n```",
                TRAILING_NEWLINES.replace(raw, "")
            ))
        }
        "blockquote" => {
            let inner = render_blocks(&el.children);
            let lines: Vec<String> = inner
                .split('\n')
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect();
            para(lines.join("\n"))
        }
        "ul" | "ol" => para(render_el_list(el)),
        "li" => para(render_list("ul", None, std::slice::from_ref(node))),
        "table" => para(render_table(el)),
        "div" => Block {
            text: render_blocks(&el.children),
            kind: Kind::Line,
        },
        _ => para(render_blocks(&el.children)),
    }
}

fn render_blocks(nodes: &[Node]) -> String {
    let refs: Vec<&Node> = nodes.iter().collect();
    render_block_refs(&refs)
}

fn render_block_refs(nodes: &[&Node]) -> String {
    let mut blocks: Vec<Block> = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, blocks: &mut Vec<Block>| {
        let text = clean_lines(run);
        run.clear();
        if !text.is_empty() {
            blocks.push(Block {
                text,
                kind: Kind::Para,
            });
        }
    };
    for node in nodes {
        if let Some(el) = block_el(node) {
            flush(&mut run, &mut blocks);
            blocks.push(block_of(node, el));
        } else {
            run.push_str(&inline_node(node));
        }
    }
    flush(&mut run, &mut blocks);
    join_blocks(&blocks)
}

static CRLF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r\n?").unwrap());
static HAS_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[A-Za-z!/][^>]*>").unwrap());

/// Converts a Targetprocess rich-text value to light Markdown for AI clients:
/// headings, bold/italic/strikethrough, code and code blocks, links, images,
/// nested bulleted and numbered lists, quotes, tables and rules. Colours, fonts
/// and alignment have no Markdown form and are dropped (Targetprocess keeps
/// them). Markdown descriptions (`<!--markdown-->…`) are returned unchanged;
/// text without tags keeps its line breaks.
pub fn html_to_text(html: Option<&str>) -> String {
    let Some(html) = html.filter(|h| !h.is_empty()) else {
        return String::new();
    };
    if MARKDOWN_MARKER.is_match(html) {
        return js_trim(&MARKDOWN_MARKER.replace(html, "")).to_string();
    }
    let normalized = CRLF.replace_all(html, "\n");
    if !HAS_TAG.is_match(&normalized) {
        return js_trim(&decode_entities(&normalized)).to_string();
    }
    render_blocks(&parse_html(&normalized))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

static LOOKS_LIKE_HTML: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(p|div|br|ul|ol|li|a|b|strong|i|em|code|pre|h[1-6]|span|table|img|blockquote)(?-u:\b)[^>]*>").unwrap()
});

/// How rich text passed to a write tool is meant: Markdown, HTML, or plain text.
/// Not given (`None`): detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichTextFormat {
    Markdown,
    Html,
    Text,
}

/// Prepares a description or comment for Targetprocess.
/// - `Markdown`: stored as Markdown (Targetprocess's `<!--markdown-->` marker is added).
/// - `Html`: sent as-is.
/// - `Text`: escaped; each line becomes a `<div>`.
/// - `None`: HTML and marked Markdown are sent as-is, anything else is treated as text.
pub fn text_to_html(text: &str, format: Option<RichTextFormat>) -> String {
    match format {
        Some(RichTextFormat::Markdown) => {
            return if MARKDOWN_MARKER.is_match(text) {
                text.to_string()
            } else {
                format!("<!--markdown-->{text}")
            };
        }
        Some(RichTextFormat::Html) => return text.to_string(),
        None if LOOKS_LIKE_HTML.is_match(text) || MARKDOWN_MARKER.is_match(text) => {
            return text.to_string();
        }
        _ => {}
    }
    CRLF.replace_all(text, "\n")
        .split('\n')
        .map(|line| {
            if js_trim(line).is_empty() {
                "<div><br></div>".to_string()
            } else {
                format!("<div>{}</div>", escape_html(line))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(html: &str) -> String {
        html_to_text(Some(html))
    }

    #[test]
    fn turns_blocks_into_lines_and_decodes_entities() {
        assert_eq!(
            t("<div>As a user&#44; I want</div><div>more&#33;</div>"),
            "As a user, I want\nmore!"
        );
    }

    #[test]
    fn renders_lists_breaks_and_links() {
        assert_eq!(
            t(
                r#"<p>Steps:</p><ul><li>one</li><li>two</li></ul>line<br/>next <a href="https://x.y/z">spec</a>"#
            ),
            "Steps:\n\n- one\n- two\n\nline\nnext [spec](https://x.y/z)"
        );
    }

    #[test]
    fn keeps_markdown_descriptions_as_markdown() {
        assert_eq!(t("<!--markdown-->**bold**\n- item"), "**bold**\n- item");
    }

    #[test]
    fn keeps_text_that_was_stored_escaped_as_the_literal_text_targetprocess_shows() {
        assert_eq!(t("&lt;p&gt;x&lt;&#47;p&gt;"), "<p>x</p>");
    }

    #[test]
    fn handles_empty_values() {
        assert_eq!(html_to_text(None), "");
        assert_eq!(t("<div></div>"), "");
    }

    #[test]
    fn decodes_latin1_entities_with_exact_case() {
        assert_eq!(
            t(
                "<div>&Ograve; &ograve; &oacute; &ntilde; &ccedil; &szlig; &frac12; &sup2; &euro;</div>"
            ),
            "Ò ò ó ñ ç ß ½ ² €"
        );
        assert_eq!(t("<div>&unknown; &amp;</div>"), "&unknown; &");
    }

    #[test]
    fn keeps_newlines_of_plain_text_and_pre_blocks() {
        assert_eq!(t("line one\nline two"), "line one\nline two");
        assert_eq!(
            t("<p>Code:</p><pre>a = 1\nb = 2</pre>"),
            "Code:\n\n```\na = 1\nb = 2\n```"
        );
    }

    #[test]
    fn round_trips_blank_paragraphs_written_by_text_to_html() {
        assert_eq!(t(&text_to_html("first\n\nsecond", None)), "first\n\nsecond");
    }

    #[test]
    fn renders_the_live_html_showcase_as_structured_markdown() {
        let md = t(include_str!("../../tests/fixtures/showcase.html"));
        for part in [
            "# Testone: style showcase",
            "**Bold**, *italic*, underlined, ~~strikethrough~~, ***all three together***, `inline code`",
            "- Second item with **bold**\n  - Nested item A\n  - Nested item B\n    - Deeply nested item\n- Third item",
            "1. Step one\n2. Step two\n   1. Sub-step 2.1\n   2. Sub-step 2.2\n3. Step three",
            "A [link with text](https://github.com/RedFoxxo/pounce)",
            "> Every write is read back to confirm it stuck.\n>\n> — the pounce README",
            "```\n{\n  \"type\": \"user story\",\n  \"name\": \"Testone\",\n  \"verified\": true\n}\n```",
            "| Feature | Status | Notes |\n| --- | --- | --- |\n| Create | **OK** | Parent inherited |",
            "\n---\n",
            "###### Heading 6",
            "Line one\nline two after a line break.",
        ] {
            assert!(md.contains(part), "missing {part:?} in:\n{md}");
        }
    }

    #[test]
    fn handles_lenient_html() {
        assert_eq!(t("<ul><li>a<li>b</ul>"), "- a\n- b");
        assert_eq!(
            t(r#"<p><a href="https://x.y/?a=1&amp;b=2" title="a > b">x</a></p>"#),
            "[x](https://x.y/?a=1&b=2)"
        );
        assert_eq!(
            t(r#"<p><img src="https://x.y/i.png" alt="diagram"></p>"#),
            "![diagram](https://x.y/i.png)"
        );
        assert_eq!(
            t(r#"<ol start="3"><li><input type="checkbox" checked> done</li></ol>"#),
            "3. [x] done"
        );
        assert_eq!(t("<p><strong>bold </strong>text</p>"), "**bold** text");
    }

    #[test]
    fn text_to_html_honours_the_format() {
        assert_eq!(
            text_to_html("# Title\n**b**", Some(RichTextFormat::Markdown)),
            "<!--markdown--># Title\n**b**"
        );
        assert_eq!(
            text_to_html("<!--markdown-->x", Some(RichTextFormat::Markdown)),
            "<!--markdown-->x"
        );
        assert_eq!(
            text_to_html("<b>x</b>", Some(RichTextFormat::Text)),
            "<div>&lt;b&gt;x&lt;/b&gt;</div>"
        );
        assert_eq!(
            text_to_html("plain <b>x", Some(RichTextFormat::Html)),
            "plain <b>x"
        );
        assert_eq!(
            text_to_html("# not markdown by default", None),
            "<div># not markdown by default</div>"
        );
    }

    /// Four times the input must take far less than sixteen times as long (quadratic).
    /// A ratio, not a time limit: unoptimized test builds on CI runners are slow.
    #[test]
    fn runs_in_linear_time_on_many_unclosed_tags() {
        let run = |n: usize| {
            let started = std::time::Instant::now();
            t(&format!("<p>{}", "<a".repeat(n)));
            t(&format!("<p>{}", "<!--".repeat(n)));
            t(&format!("<p>{}", "<a \"".repeat(n)));
            started.elapsed().as_secs_f64()
        };
        let (small, large) = (run(10_000), run(40_000));
        assert!(large < small * 8.0, "10k: {small:.3}s, 40k: {large:.3}s");
    }

    /// format.test.ts's "untrusted payload keys" case for `customFields` (shape.rs is not this port's file).
    #[test]
    fn custom_fields_keeps_a_field_named_proto() {
        let fields = serde_json::json!([{ "Name": "__proto__", "Value": "x" }]);
        assert_eq!(
            crate::format::shape::custom_fields(&fields).to_string(),
            r#"{"__proto__":"x"}"#
        );
    }
}
