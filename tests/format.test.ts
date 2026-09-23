import { describe, expect, it } from 'vitest'
import { htmlToText, textToHtml } from '../src/format/html.js'

describe('htmlToText', () => {
  it('turns blocks into lines and decodes entities', () => {
    expect(htmlToText('<div>As a user&#44; I want</div><div>more&#33;</div>')).toBe('As a user, I want\nmore!')
  })

  it('renders lists, breaks and links', () => {
    expect(htmlToText('<p>Steps:</p><ul><li>one</li><li>two</li></ul>line<br/>next <a href="https://x.y/z">spec</a>')).toBe(
      'Steps:\n- one\n- two\nline\nnext spec (https://x.y/z)',
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
    expect(htmlToText('<p>Code:</p><pre>a = 1\nb = 2</pre>')).toBe('Code:\na = 1\nb = 2')
  })

  it('round-trips blank paragraphs written by textToHtml', () => {
    expect(htmlToText(textToHtml('first\n\nsecond'))).toBe('first\n\nsecond')
  })
})
