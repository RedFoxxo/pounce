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

/** Placeholders while converting: a block boundary, and a newline to keep (inside <pre>). */
const BLOCK = '\u0001'
const KEEP_NL = '\u0002'

const MARKDOWN_MARKER = /^\s*<!--\s*markdown\s*-->\s*/i

/**
 * Converts a Targetprocess rich-text value to plain text: block elements become
 * line breaks, list items get bullets, links keep their target, entities are
 * decoded. Markdown descriptions (`<!--markdown-->…`) are returned as markdown.
 */
export function htmlToText(html: string | null | undefined): string {
  if (!html) return ''
  if (MARKDOWN_MARKER.test(html)) return html.replace(MARKDOWN_MARKER, '').trim()
  const normalized = html.replace(/\r\n?/g, '\n')
  // Plain text (no tags), e.g. written by another client: keep its line breaks.
  if (!/<[A-Za-z!/][^>]*>/.test(normalized)) return decodeEntities(normalized).trim()

  let text = normalized
    .replace(/<(script|style)\b[\s\S]*?<\/\1>/gi, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/(<pre\b[^>]*>)([\s\S]*?)(<\/pre>)/gi, (_m, open: string, body: string, close: string) => open + body.replace(/\n/g, KEEP_NL) + close)
    .replace(/\n/g, ' ')
    .replace(/<a\b[^>]*href\s*=\s*("([^"]*)"|'([^']*)')[^>]*>([\s\S]*?)<\/a>/gi, (_m, _q, dq?: string, sq?: string, inner?: string) => {
      const href = dq ?? sq ?? ''
      const label = (inner ?? '').replace(/<[^>]*>/g, '').trim()
      return !label || label === href ? href : `${label} (${href})`
    })
    .replace(/<img\b[^>]*alt\s*=\s*"([^"]*)"[^>]*>/gi, '[image: $1]')
    .replace(/<img\b[^>]*>/gi, '[image]')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<li\b[^>]*>/gi, `${BLOCK}- `)
    .replace(/<\/(p|div|h[1-6]|li|ul|ol|table|tr|blockquote|pre)>/gi, BLOCK)
    .replace(/<(p|div|h[1-6]|ul|ol|table|tr|blockquote|pre)\b[^>]*>/gi, BLOCK)
    .replace(/<\/t[dh]>/gi, '\t')
    .replace(/<[^>]*>/g, '')
    // adjacent block boundaries make one line break; explicit <br> breaks are kept
    .replace(new RegExp(`${BLOCK}[ \\t${BLOCK}]*`, 'g'), '\n')
    .replace(new RegExp(KEEP_NL, 'g'), '\n')

  text = decodeEntities(text)
  return text
    .split('\n')
    .map((line) => line.replace(/[ \t\u00a0]+/g, ' ').trim())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

const LOOKS_LIKE_HTML = /<\/?(p|div|br|ul|ol|li|a|b|strong|i|em|code|pre|h[1-6]|span|table|img|blockquote)\b[^>]*>/i

/**
 * Prepares a description for Targetprocess. HTML is sent as-is (never
 * escaped twice); plain text is escaped and each line becomes a `<div>`.
 */
export function textToHtml(text: string): string {
  if (LOOKS_LIKE_HTML.test(text) || MARKDOWN_MARKER.test(text)) return text
  return text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => (line.trim() ? `<div>${escapeHtml(line)}</div>` : '<div><br></div>'))
    .join('')
}
