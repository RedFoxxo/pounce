/** Latin-1 named entities (U+00A0–U+00FF), in code point order. */
const LATIN1 =
  'nbsp iexcl cent pound curren yen brvbar sect uml copy ordf laquo not shy reg macr deg plusmn sup2 sup3 acute micro para middot cedil sup1 ordm raquo frac14 frac12 frac34 iquest Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml'.split(' ')

/** Named entities, exact case (`&Ograve;` and `&ograve;` differ). */
const NAMED: Record<string, string> = {
  ...Object.fromEntries(LATIN1.map((name, i) => [name, String.fromCharCode(0xa0 + i)])),
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  OElig: 'Œ',
  oelig: 'œ',
  Scaron: 'Š',
  scaron: 'š',
  Yuml: 'Ÿ',
  fnof: 'ƒ',
  circ: 'ˆ',
  tilde: '˜',
  ensp: '\u2002',
  emsp: '\u2003',
  thinsp: '\u2009',
  zwnj: '\u200c',
  zwj: '\u200d',
  ndash: '–',
  mdash: '—',
  lsquo: '‘',
  rsquo: '’',
  sbquo: '‚',
  ldquo: '“',
  rdquo: '”',
  bdquo: '„',
  dagger: '†',
  Dagger: '‡',
  bull: '•',
  hellip: '…',
  permil: '‰',
  lsaquo: '‹',
  rsaquo: '›',
  euro: '€',
  trade: '™',
  larr: '←',
  uarr: '↑',
  rarr: '→',
  darr: '↓',
  harr: '↔',
  check: '✓',
}

export function decodeEntities(text: string): string {
  return text.replace(/&(#[xX][0-9a-fA-F]+|#\d+|[A-Za-z][A-Za-z0-9]*);/g, (whole, code: string) => {
    if (code[0] === '#') {
      const n = code[1] === 'x' || code[1] === 'X' ? parseInt(code.slice(2), 16) : Number(code.slice(1))
      return Number.isFinite(n) && n > 0 && n <= 0x10ffff ? String.fromCodePoint(n) : whole
    }
    return NAMED[code] ?? whole
  })
}

const MARKDOWN_MARKER = /^\s*<!--\s*markdown\s*-->\s*/i

// ---------------------------------------------------------------- HTML → light Markdown

interface Element {
  type: 'el'
  tag: string
  attrs: Record<string, string>
  children: HtmlNode[]
}
type HtmlNode = { type: 'text'; text: string } | Element

const VOID = new Set(['br', 'hr', 'img', 'input', 'meta', 'link', 'col', 'wbr', 'area', 'base', 'source'])
const SKIP = new Set(['script', 'style', 'head', 'title'])
/** Opening one of these closes an open element of the same kind (lenient HTML). */
const SELF_CLOSING_SIBLINGS = new Set(['li', 'p', 'tr', 'td', 'th', 'dt', 'dd'])
const BLOCK = new Set([
  'p', 'div', 'section', 'article', 'header', 'footer', 'main', 'aside', 'nav', 'figure', 'figcaption', 'address', 'center',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'blockquote', 'pre', 'table', 'thead', 'tbody',
  'tfoot', 'tr', 'td', 'th', 'caption', 'hr',
])

function parseAttrs(raw: string): Record<string, string> {
  const attrs: Record<string, string> = {}
  for (const m of raw.matchAll(/([A-Za-z_:][\w:.-]*)\s*(?:=\s*("([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g)) {
    attrs[(m[1] as string).toLowerCase()] = decodeEntities(m[3] ?? m[4] ?? m[5] ?? '')
  }
  return attrs
}

function parseHtml(html: string): HtmlNode[] {
  const root: Element = { type: 'el', tag: '#root', attrs: {}, children: [] }
  const stack: Element[] = [root]
  const top = () => stack[stack.length - 1] as Element
  let skipping: string | undefined
  for (const m of html.matchAll(/<!--[\s\S]*?-->|<\/?([A-Za-z][A-Za-z0-9]*)\b((?:[^>"']|"[^"]*"|'[^']*')*)>|[^<]+|</g)) {
    const token = m[0]
    const tagName = m[1]?.toLowerCase()
    if (skipping) {
      if (tagName === skipping && token.startsWith('</')) skipping = undefined
      continue
    }
    if (token.startsWith('<!--')) continue
    if (!tagName) {
      top().children.push({ type: 'text', text: token })
      continue
    }
    if (token.startsWith('</')) {
      const at = stack.map((e) => e.tag).lastIndexOf(tagName)
      if (at > 0) stack.length = at
      continue
    }
    if (SKIP.has(tagName)) {
      if (!/\/\s*>$/.test(token)) skipping = tagName
      continue
    }
    if (SELF_CLOSING_SIBLINGS.has(tagName) && top().tag === tagName) stack.pop()
    const el: Element = { type: 'el', tag: tagName, attrs: parseAttrs(m[2] ?? ''), children: [] }
    top().children.push(el)
    if (!VOID.has(tagName) && !/\/\s*>$/.test(token)) stack.push(el)
  }
  return root.children
}

function isBlock(node: HtmlNode): boolean {
  return node.type === 'el' && BLOCK.has(node.tag)
}

/** Raw text of a subtree, entities decoded, whitespace kept (for code). */
function rawText(nodes: HtmlNode[]): string {
  return nodes.map((n) => (n.type === 'text' ? decodeEntities(n.text) : n.tag === 'br' ? '\n' : rawText(n.children))).join('')
}

/** Wraps the non-space core of `text` in a Markdown mark, keeping surrounding spaces outside. */
function mark(text: string, left: string, right = left): string {
  const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(text) as RegExpExecArray
  return m[2] ? `${m[1]}${left}${m[2]}${right}${m[3]}` : text
}

function inline(nodes: HtmlNode[]): string {
  return nodes.map(inlineNode).join('')
}

function inlineNode(node: HtmlNode): string {
  if (node.type === 'text') return decodeEntities(node.text).replace(/[ \t\n\r\f]+/g, ' ')
  const inner = () => inline(node.children)
  switch (node.tag) {
    case 'br':
      return '\n'
    case 'strong':
    case 'b':
      return mark(inner(), '**')
    case 'em':
    case 'i':
      return mark(inner(), '*')
    case 's':
    case 'del':
    case 'strike':
      return mark(inner(), '~~')
    case 'code':
    case 'kbd':
    case 'samp':
      return mark(rawText(node.children).replace(/\n/g, ' '), '`')
    case 'a': {
      const href = node.attrs.href ?? ''
      const label = inner().trim()
      if (!href) return label
      return !label || label === href ? href : `[${label}](${href})`
    }
    case 'img': {
      const src = node.attrs.src ?? ''
      return src ? `![${node.attrs.alt ?? ''}](${src})` : node.attrs.alt ? `[image: ${node.attrs.alt}]` : '[image]'
    }
    case 'input':
      return node.attrs.type === 'checkbox' ? ('checked' in node.attrs ? '[x] ' : '[ ] ') : ''
    default:
      // A block inside inline context (e.g. <p> in a <span>): keep it readable on its own lines.
      return isBlock(node) ? `\n${renderBlocks(node.children)}\n` : inner()
  }
}

interface Block {
  text: string
  /** `line` blocks (Targetprocess's one-div-per-line) join with a single newline; others with a blank line. */
  kind: 'line' | 'para'
}

function cleanLines(text: string): string {
  return text
    .split('\n')
    .map((l) => l.replace(/[ \t\u00a0]+/g, ' ').trim())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

function joinBlocks(blocks: Block[]): string {
  let out = ''
  let prev: Block | undefined
  for (const b of blocks) {
    if (b.kind === 'para' && !b.text) continue
    if (prev) out += prev.kind === 'line' && b.kind === 'line' ? '\n' : '\n\n'
    out += b.text
    prev = b
  }
  return out.replace(/\n{3,}/g, '\n\n').trim()
}

function indent(text: string, by: string): string {
  return text
    .split('\n')
    .map((l) => (l ? by + l : l))
    .join('\n')
}

function renderList(list: Element): string {
  const ordered = list.tag === 'ol'
  let n = Number.parseInt(list.attrs.start ?? '1', 10) || 1
  const items: string[] = []
  for (const child of list.children) {
    if (child.type === 'text') {
      if (child.text.trim()) items.push(`- ${cleanLines(inline([child]))}`)
      continue
    }
    if (child.tag === 'ul' || child.tag === 'ol') {
      items.push(indent(renderList(child), '  '))
      continue
    }
    const marker = ordered ? `${n++}.` : '-'
    const nested = child.tag === 'li' ? child.children.filter((c) => c.type === 'el' && (c.tag === 'ul' || c.tag === 'ol')) : []
    const rest = child.tag === 'li' ? child.children.filter((c) => !nested.includes(c)) : [child]
    const body = renderBlocks(rest)
    const pad = ' '.repeat(marker.length + 1)
    const [first = '', ...more] = body.split('\n')
    let item = `${marker} ${first}`.trimEnd() + (more.length ? `\n${indent(more.join('\n'), pad)}` : '')
    for (const sub of nested) item += `\n${indent(renderList(sub as Element), pad)}`
    items.push(item)
  }
  return items.join('\n')
}

function renderTable(table: Element): string {
  const rows: Element[] = []
  const collect = (nodes: HtmlNode[]) => {
    for (const n of nodes) {
      if (n.type !== 'el') continue
      if (n.tag === 'tr') rows.push(n)
      else if (['thead', 'tbody', 'tfoot'].includes(n.tag)) collect(n.children)
    }
  }
  collect(table.children)
  const cells = rows.map((r) =>
    r.children
      .filter((c): c is Element => c.type === 'el' && (c.tag === 'td' || c.tag === 'th'))
      .map((c) => cleanLines(renderBlocks(c.children)).replace(/\n+/g, ' ').replace(/\|/g, '\\|')),
  )
  const width = Math.max(0, ...cells.map((r) => r.length))
  if (!width) return ''
  const line = (r: string[]) => `| ${Array.from({ length: width }, (_, i) => r[i] ?? '').join(' | ')} |`
  const [head = [], ...body] = cells
  return [line(head), `|${' --- |'.repeat(width)}`, ...body.map(line)].join('\n')
}

function blockOf(el: Element): Block {
  switch (el.tag) {
    case 'h1':
    case 'h2':
    case 'h3':
    case 'h4':
    case 'h5':
    case 'h6': {
      const text = cleanLines(inline(el.children)).replace(/\n+/g, ' ')
      return { text: text ? `${'#'.repeat(Number(el.tag[1]))} ${text}` : '', kind: 'para' }
    }
    case 'hr':
      return { text: '---', kind: 'para' }
    case 'pre': {
      const code = el.children.length === 1 && el.children[0]?.type === 'el' && el.children[0].tag === 'code' ? el.children[0] : el
      const lang = /(?:^|\s)(?:language|lang)-([\w+-]+)/.exec(code.attrs.class ?? '')?.[1] ?? ''
      return { text: `\`\`\`${lang}\n${rawText(code.children).replace(/^\n/, '').replace(/\n+$/, '')}\n\`\`\``, kind: 'para' }
    }
    case 'blockquote': {
      const inner = renderBlocks(el.children)
      return { text: inner.split('\n').map((l) => (l ? `> ${l}` : '>')).join('\n'), kind: 'para' }
    }
    case 'ul':
    case 'ol':
      return { text: renderList(el), kind: 'para' }
    case 'li':
      return { text: renderList({ type: 'el', tag: 'ul', attrs: {}, children: [el] }), kind: 'para' }
    case 'table':
      return { text: renderTable(el), kind: 'para' }
    case 'div':
      return { text: renderBlocks(el.children), kind: 'line' }
    default:
      return { text: renderBlocks(el.children), kind: 'para' }
  }
}

function renderBlocks(nodes: HtmlNode[]): string {
  const blocks: Block[] = []
  let run: HtmlNode[] = []
  const flush = () => {
    if (!run.length) return
    const text = cleanLines(inline(run))
    run = []
    if (text) blocks.push({ text, kind: 'para' })
  }
  for (const node of nodes) {
    if (isBlock(node)) {
      flush()
      blocks.push(blockOf(node as Element))
    } else {
      run.push(node)
    }
  }
  flush()
  return joinBlocks(blocks)
}

/**
 * Converts a Targetprocess rich-text value to light Markdown for AI clients:
 * headings, bold/italic/strikethrough, code and code blocks, links, images,
 * nested bulleted and numbered lists, quotes, tables and rules. Colours, fonts
 * and alignment have no Markdown form and are dropped (Targetprocess keeps
 * them). Markdown descriptions (`<!--markdown-->…`) are returned unchanged;
 * text without tags keeps its line breaks.
 */
export function htmlToText(html: string | null | undefined): string {
  if (!html) return ''
  if (MARKDOWN_MARKER.test(html)) return html.replace(MARKDOWN_MARKER, '').trim()
  const normalized = html.replace(/\r\n?/g, '\n')
  if (!/<[A-Za-z!/][^>]*>/.test(normalized)) return decodeEntities(normalized).trim()
  return renderBlocks(parseHtml(normalized))
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

const LOOKS_LIKE_HTML = /<\/?(p|div|br|ul|ol|li|a|b|strong|i|em|code|pre|h[1-6]|span|table|img|blockquote)\b[^>]*>/i

/** How rich text passed to a write tool is meant: Markdown, HTML, plain text, or detected (default). */
export type RichTextFormat = 'markdown' | 'html' | 'text'

/**
 * Prepares a description or comment for Targetprocess.
 * - `markdown`: stored as Markdown (Targetprocess's `<!--markdown-->` marker is added).
 * - `html`: sent as-is.
 * - `text`: escaped; each line becomes a `<div>`.
 * - not given: HTML and marked Markdown are sent as-is, anything else is treated as text.
 */
export function textToHtml(text: string, format?: RichTextFormat): string {
  if (format === 'markdown') return MARKDOWN_MARKER.test(text) ? text : `<!--markdown-->${text}`
  if (format === 'html') return text
  if (format === undefined && (LOOKS_LIKE_HTML.test(text) || MARKDOWN_MARKER.test(text))) return text
  return text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => (line.trim() ? `<div>${escapeHtml(line)}</div>` : '<div><br></div>'))
    .join('')
}
