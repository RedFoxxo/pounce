const NAMED: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: '\u00a0',
  ndash: '–',
  mdash: '—',
  hellip: '…',
  laquo: '«',
  raquo: '»',
  lsquo: '‘',
  rsquo: '’',
  ldquo: '“',
  rdquo: '”',
  bull: '•',
  middot: '·',
  copy: '©',
  reg: '®',
  trade: '™',
  euro: '€',
  times: '×',
  deg: '°',
  agrave: 'à',
  egrave: 'è',
  eacute: 'é',
  igrave: 'ì',
  ograve: 'ò',
  ugrave: 'ù',
  Agrave: 'À',
  Egrave: 'È',
  Eacute: 'É',
}

export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, code: string) => {
    if (code[0] === '#') {
      const n = code[1] === 'x' || code[1] === 'X' ? parseInt(code.slice(2), 16) : Number(code.slice(1))
      return Number.isFinite(n) && n > 0 && n <= 0x10ffff ? String.fromCodePoint(n) : whole
    }
    return NAMED[code] ?? NAMED[code.toLowerCase()] ?? whole
  })
}

/** Placeholder for a block boundary while converting. */
const BLOCK = '\u0001'

const MARKDOWN_MARKER = /^\s*<!--\s*markdown\s*-->\s*/i

/**
 * Converts a Targetprocess rich-text value to plain text: block elements become
 * line breaks, list items get bullets, links keep their target, entities are
 * decoded. Markdown descriptions (`<!--markdown-->…`) are returned as markdown.
 */
export function htmlToText(html: string | null | undefined): string {
  if (!html) return ''
  if (MARKDOWN_MARKER.test(html)) return html.replace(MARKDOWN_MARKER, '').trim()

  let text = html
    .replace(/\r\n?/g, '\n')
    .replace(/<(script|style)\b[\s\S]*?<\/\1>/gi, '')
    .replace(/<!--[\s\S]*?-->/g, '')
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
    // adjacent block boundaries make one line break
    .replace(new RegExp(`${BLOCK}[\\s${BLOCK}]*`, 'g'), '\n')

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
