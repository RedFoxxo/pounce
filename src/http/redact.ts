const TOKEN_PARAM = /([?&](?:access_token|token)=)[^&#\s"']*/gi

/** Removes the token from a URL, log line or response body. */
export function redact(text: string, token?: string): string {
  let out = text.replace(TOKEN_PARAM, '$1***')
  if (token && token.length >= 4) out = out.split(token).join('***')
  if (token && token.length >= 4) {
    const encoded = encodeURIComponent(token)
    if (encoded !== token) out = out.split(encoded).join('***')
  }
  return out
}
