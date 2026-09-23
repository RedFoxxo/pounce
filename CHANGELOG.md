# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`package.json` and `src/version.ts` (a test keeps them equal) and is reported to
MCP clients in the server handshake.

## 1.0.1 — 2026-09-23

- Markdown descriptions and comments: `write_create_card`, `write_update_card`
  and `write_comment` take `format: "markdown"` (also `html` or `text`), and
  pounce stores the text as a Targetprocess Markdown description. Before, plain
  Markdown was escaped and showed its `**` and `#` literally.
- Rich-text fields are read back as light Markdown instead of flattened text:
  headings, bold/italic/strikethrough, code and code blocks, links, images,
  nested bulleted and numbered lists, quotes, tables and rules survive.
  Colours, fonts and alignment have no Markdown form and are dropped from the
  read-back only; Targetprocess keeps them.
- After a rename, `write_update_card` shows the new name and `renamedFrom`.

## 1.0.0 — 2026-09-23

First release.

- 58 tools in four permission tiers (`read_`, `write_`, `delete_`, `admin_`).
- Generic tools driven by the instance's own API metadata, with a committed
  snapshot as fallback: every resource, operation and collection is reachable.
- Workflow tools that take names instead of ids and enforce the team rules:
  effort per role with rollup reporting, default assignments cleared on
  creation (including late ones), parent side effects reported, projects
  inherited, every write read back.
- REST v2 queries, change history, RESTful storage, conversions, deleted items,
  attachment upload and undelete.
- Time logging and reading use standard Time entries, or a custom `TimeRecord`
  type on instances that track time that way (chosen per process).
- Verified end to end against a live Targetprocess instance.
- Setup instructions for opencode, Claude Code and other MCP clients.
