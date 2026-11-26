# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`Cargo.toml` and is reported to MCP clients in the server handshake.

## 2.0.0 — 2026-11-26

The same server, rewritten in Rust: one executable without Node.js, about 7 MB
of memory when idle and 17 MB with the API metadata loaded, instead of about
79 MB plus `npx`'s own process (about 54 MB).

- **Breaking: installed as a program, not through npm.** Each release on GitHub
  has binaries for Linux (x64, arm64; static, any distribution), macOS (Apple
  silicon, Intel) and Windows (x64), with `SHA256SUMS`. Clients start `pounce`
  instead of `npx -y @redfoxxo/pounce@1`; the npm package stays at 1.2.0. See
  "Install" in the README.
- **Install scripts:** `curl -fsSL …/install.sh | sh` (Linux, macOS) and
  `irm …/install.ps1 | iex` (Windows) install or update pounce and add it to
  your PATH.
- **Claude Desktop:** each release has `pounce.mcpb`, a one-click bundle that
  asks for the instance URL, the access token and the optional settings.
- **MCP Registry:** listed as `io.github.RedFoxxo/pounce`, updated by every
  release.
- From source: `cargo install pounce-mcp` (the name `pounce` is taken on
  crates.io; the program is still `pounce`). A Dockerfile builds a container
  image.
- Tools, parameters, descriptions, answers and environment variables are
  unchanged: `tools/list` is the 1.2.0 list, and the tool calls recorded from
  the 1.2.0 tests send the same requests and give the same answers.
- `pounce --version` and `pounce --help`.
- Starts in milliseconds and needs no network until the first tool call (`npx`
  checked the registry on every start).

## 1.2.0 — 2026-11-19

- No requests at startup: the instance's API metadata is read lazily, one
  resource at a time, the first time a tool needs it, instead of about 270
  requests every time the server starts. Concurrent calls share one load. A
  failed load is retried on the next call (the built-in snapshot stands in
  meanwhile) instead of leaving the snapshot in use until restart.
- Smaller tool list: pounce answers `tools/list` without the SDK's `$schema`,
  `execution` and default-only schema keywords, marks only read tools
  (`readOnlyHint`), and publishes name-or-id parameters as plain strings
  (numbers are still accepted). The serialized list shrinks from 23,400 to
  16,200 bytes for `cards` and from 41,500 to 29,800 for all 58 tools (about
  4,050 and 7,450 tokens at 4 bytes per token).
- `write_create_card` and `write_update_card` no longer echo the description
  you just sent (it is still verified); `read_card` omits an empty
  `customFields`.
- `read_my_work` no longer loads the whole user list to look up the current
  user a second time.
- A `POUNCE_TOOLSETS` with no names (e.g. `,`) is now a startup error instead of
  a server with no tools.
- Converting a pathological description (many unclosed tags or comments) to
  Markdown is linear instead of quadratic; a 60 KB one took seconds before.
- A `__proto__` key in a Targetprocess payload or a custom field named
  `__proto__` is kept as data.
- The built-in catalog snapshot drops field descriptions and the `canGet` flag,
  which nothing used: 1.38 MB → 0.92 MB in the package.
- Dependencies: `@modelcontextprotocol/sdk` 1.32.2 and `zod` 4.6.7 (security
  advisories), `vitest` 5.1.1; `npm audit` reports no vulnerabilities.
- README: setup follows the current opencode config (`mcp.servers`,
  `permissions` rules); the stale version line is gone. CI runs typecheck, tests
  and build on `main`, `stable` and pull requests.

## 1.1.0 — 2026-11-06

- Toolsets: `POUNCE_TOOLSETS` (`cards`, `raw`, `admin`, comma-separated) limits
  the tools pounce registers. Unset, all 58 are registered as before;
  `POUNCE_TOOLSETS=cards` registers the 34 card tools.
- Shorter tool descriptions: one sentence each, with the rules that matter
  before a call moved into the parameter they concern. Listing all 58 tools
  now costs about 8,000 tokens instead of 11,200 (about 4,450 for `cards`).
- Id parameters publish a compact `{"type":"number"}` schema; numeric strings
  are still accepted.

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
