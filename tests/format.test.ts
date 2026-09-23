import { describe, expect, it } from 'vitest'
import { htmlToText } from '../src/format/html.js'

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
})
