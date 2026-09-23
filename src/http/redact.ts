const TOKEN_PARAM = /([?&](?:access_token|token)=)[^&#\s"'\\]*/gi

/** Replaces the exact token, raw and URL-encoded. Safe on JSON: it never touches other characters. */
export function redactToken(text: string, token?: string): string {
  if (!token || token.length < 4) return text
  let out = text.split(token).join('***')
  const encoded = encodeURIComponent(token)
  if (encoded !== token) out = out.split(encoded).join('***')
  return out
}

/**
 * Removes the token from a URL or log line: the exact token, plus any
 * `access_token=`/`token=` query value. Not for response bodies, where the
 * parameter pattern would rewrite user content (links) and could corrupt JSON.
 */
export function redact(text: string, token?: string): string {
  return redactToken(text.replace(TOKEN_PARAM, '$1***'), token)
}
