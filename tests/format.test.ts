import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { htmlToText, textToHtml } from '../src/format/html.js'

describe('htmlToText', () => {
  it('turns blocks into lines and decodes entities', () => {
    expect(htmlToText('<div>As a user&#44; I want</div><div>more&#33;</div>')).toBe('As a user, I want\nmore!')
  })

  it('renders lists, breaks and links', () => {
    expect(htmlToText('<p>Steps:</p><ul><li>one</li><li>two</li></ul>line<br/>next <a href="https://x.y/z">spec</a>')).toBe(
      'Steps:\n\n- one\n- two\n\nline\nnext [spec](https://x.y/z)',
    )
  })

  it('keeps markdown descriptions as markdown', () => {
    expect(htmlToText('<!--markdown-->**bold**\n- item')).toBe('**bold**\n- item')
  })

  it('keeps text that was stored escaped as the literal text Targetprocess shows', () => {
    expect(htmlToText('&lt;p&gt;x&lt;&#47;p&gt;')).toBe('<p>x</p>')
  })

  it('handles empty values', () => {
    expect(htmlToText(null)).toBe('')
    expect(htmlToText('<div></div>')).toBe('')
  })

  it('decodes Latin-1 entities with exact case', () => {
    expect(htmlToText('<div>&Ograve; &ograve; &oacute; &ntilde; &ccedil; &szlig; &frac12; &sup2; &euro;</div>')).toBe('Ò ò ó ñ ç ß ½ ² €')
    expect(htmlToText('<div>&unknown; &amp;</div>')).toBe('&unknown; &')
  })

  it('keeps newlines of plain text and <pre> blocks', () => {
    expect(htmlToText('line one\nline two')).toBe('line one\nline two')
    expect(htmlToText('<p>Code:</p><pre>a = 1\nb = 2</pre>')).toBe('Code:\n\n```\na = 1\nb = 2\n```')
  })

  it('round-trips blank paragraphs written by textToHtml', () => {
    expect(htmlToText(textToHtml('first\n\nsecond'))).toBe('first\n\nsecond')
  })

  it('renders the live HTML showcase as structured Markdown', () => {
    const md = htmlToText(readFileSync(new URL('./fixtures/showcase.html', import.meta.url), 'utf8'))
    expect(md).toContain('# Testone: style showcase')
    expect(md).toContain('**Bold**, *italic*, underlined, ~~strikethrough~~, ***all three together***, `inline code`')
    expect(md).toContain('- Second item with **bold**\n  - Nested item A\n  - Nested item B\n    - Deeply nested item\n- Third item')
    expect(md).toContain('1. Step one\n2. Step two\n   1. Sub-step 2.1\n   2. Sub-step 2.2\n3. Step three')
    expect(md).toContain('A [link with text](https://github.com/RedFoxxo/pounce)')
    expect(md).toContain('> Every write is read back to confirm it stuck.\n>\n> — the pounce README')
    expect(md).toContain('```\n{\n  "type": "user story",\n  "name": "Testone",\n  "verified": true\n}\n```')
    expect(md).toContain('| Feature | Status | Notes |\n| --- | --- | --- |\n| Create | **OK** | Parent inherited |')
    expect(md).toContain('\n---\n')
    expect(md).toContain('###### Heading 6')
    expect(md).toContain('Line one\nline two after a line break.')
  })

  it('handles lenient HTML: unclosed items, attributes with >, images, checkboxes', () => {
    expect(htmlToText('<ul><li>a<li>b</ul>')).toBe('- a\n- b')
    expect(htmlToText('<p><a href="https://x.y/?a=1&amp;b=2" title="a > b">x</a></p>')).toBe('[x](https://x.y/?a=1&b=2)')
    expect(htmlToText('<p><img src="https://x.y/i.png" alt="diagram"></p>')).toBe('![diagram](https://x.y/i.png)')
    expect(htmlToText('<ol start="3"><li><input type="checkbox" checked> done</li></ol>')).toBe('3. [x] done')
    expect(htmlToText('<p><strong>bold </strong>text</p>')).toBe('**bold** text')
  })

  it('textToHtml honours the format', () => {
    expect(textToHtml('# Title\n**b**', 'markdown')).toBe('<!--markdown--># Title\n**b**')
    expect(textToHtml('<!--markdown-->x', 'markdown')).toBe('<!--markdown-->x')
    expect(textToHtml('<b>x</b>', 'text')).toBe('<div>&lt;b&gt;x&lt;/b&gt;</div>')
    expect(textToHtml('plain <b>x', 'html')).toBe('plain <b>x')
    expect(textToHtml('# not markdown by default')).toBe('<div># not markdown by default</div>')
  })
})
