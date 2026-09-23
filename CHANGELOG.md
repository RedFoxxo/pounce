# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`package.json` and `src/version.ts` (a test keeps them equal) and is reported to
MCP clients in the server handshake.

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
