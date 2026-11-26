# pounce 🦊

[![CI](https://github.com/RedFoxxo/pounce/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/RedFoxxo/pounce/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/RedFoxxo/pounce)](https://github.com/RedFoxxo/pounce/releases/latest)
[![MCP Registry](https://img.shields.io/badge/MCP_Registry-io.github.RedFoxxo%2Fpounce-blue)](https://registry.modelcontextprotocol.io/v0/servers?search=io.github.RedFoxxo/pounce)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)](#install)
[![License: MIT](https://img.shields.io/github/license/RedFoxxo/pounce)](LICENSE)

An MCP server for [Targetprocess](https://www.ibm.com/products/targetprocess): lets AI
assistants read and manage your cards through the Targetprocess REST API.
Tested against Targetprocess 2609.2 (Pro edition). Releases are listed in
[CHANGELOG.md](CHANGELOG.md).

pounce is one small executable: no Node.js or other runtime, about 7 MB of
memory when idle and about 17 MB once it has read the instance's API metadata.

- **Full coverage.** If Targetprocess exposes an operation, pounce can perform it.
  Generic tools are driven by your instance's own API metadata, so custom entity
  types and future Targetprocess versions are covered automatically.
- **Tools that fit how teams work.** Workflow tools accept names instead of ids
  ("move #36400 to Coded", "assign Foxxo as Developer") and apply the team's
  rules: effort is booked per role, default assignees are cleared on creation,
  and side effects on parent cards are reported.
- **Honest results.** Every list is fully paged, every error carries
  Targetprocess's own message, and every write is read back to confirm it stuck.
- **Simple permissions.** Tools are grouped by tier, so an MCP client needs one
  rule per tier.

## Requirements

- Linux (x64 or arm64), macOS (Apple silicon or Intel) or Windows (x64)
- A Targetprocess access token: in Targetprocess, open your profile →
  **Access Tokens** → create one

## Install

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/RedFoxxo/pounce/main/install.sh | sh
```

Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/RedFoxxo/pounce/main/install.ps1 | iex
```

The script downloads the file for your system from the
[latest release](https://github.com/RedFoxxo/pounce/releases/latest), checks
it against the release's `SHA256SUMS`, and installs it as `pounce` in
`~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\pounce`). If that folder is
not on your `PATH` yet, it adds it: to `~/.profile` (and `~/.zprofile` on
macOS), or to your user `PATH` on Windows. Log out and back in on Linux and
macOS, or open a new terminal on Windows, then check with `pounce --version`.

Run the same line again to update. Settings go to the script: `POUNCE_VERSION`
installs a given release, `POUNCE_INSTALL_DIR` uses another folder, and
`POUNCE_NO_MODIFY_PATH=1` leaves your `PATH` alone, e.g.
`curl -fsSL …/install.sh | POUNCE_VERSION=2.0.0 sh`, or in PowerShell
`$env:POUNCE_VERSION = '2.0.0'` before `irm … | iex`.

### Claude Desktop

Download `pounce.mcpb` from the
[latest release](https://github.com/RedFoxxo/pounce/releases/latest) and open
it, or drag it into **Settings → Extensions**. Claude Desktop asks for your
Targetprocess URL and access token (kept in the system's secure storage), plus
the optional default project, default team and toolsets, and runs pounce from
the bundle; nothing else to install. pounce is also listed in the
[MCP Registry](https://registry.modelcontextprotocol.io/v0/servers?search=io.github.RedFoxxo/pounce)
as `io.github.RedFoxxo/pounce`.

### By hand

Download the file for your system from the latest release into a folder on your
`PATH`, rename it to `pounce` (`pounce.exe` on Windows), and on Linux and macOS
make it executable (`chmod +x pounce`). On macOS, a file downloaded with a
browser needs `xattr -d com.apple.quarantine pounce` once before it starts.

| System | File |
|---|---|
| Linux x64 | `pounce-linux-x64` |
| Linux arm64 | `pounce-linux-arm64` |
| macOS, Apple silicon | `pounce-macos-arm64` |
| macOS, Intel | `pounce-macos-x64` |
| Windows x64, also Windows on ARM | `pounce-windows-x64.exe` |

To build from source instead (needs Rust): `cargo install pounce-mcp --locked`
(the crate is `pounce-mcp`; the program is `pounce`), or from the repository:
`cargo install --git https://github.com/RedFoxxo/pounce --tag v2.0.0 --locked`.
Or as a container image: `docker build -t pounce .` in a clone, then start it
with `docker run -i --rm -e TP_BASE_URL -e TP_TOKEN pounce`.

**Coming from 1.x?** pounce 1.x ran through `npx` from the npm package
`@redfoxxo/pounce`, which gets no new versions. Install pounce as above, then
change your client's `"command": "npx", "args": ["-y", "@redfoxxo/pounce@1"]`
to `"command": "pounce"` (opencode: `"command": ["pounce"]`). Tools, settings
and permission rules stay the same.

## Setup

pounce reads its settings from environment variables, which your MCP client
passes to it:

| Variable | Required | Purpose |
|---|---|---|
| `TP_BASE_URL` | yes | Your instance, e.g. `https://yourcompany.tpondemand.com` (no `/api/...`). `https` only, except for `localhost` |
| `TP_TOKEN` | yes | Your access token. Never logged |
| `TP_DEFAULT_PROJECT_ID` | no | Project for new cards that have no parent to inherit one from |
| `TP_DEFAULT_TEAM_ID` | no | Team for new cards when none is given |
| `POUNCE_TOOLSETS` | no | Tool groups to expose, comma-separated: `cards`, `raw`, `admin` (default: all three). See [Tools](#tools) |

Keep the token out of config files: export it in your shell profile
(`export TP_TOKEN=...`) and let the client pass it through, as the examples
below do.

pounce sends nothing to Targetprocess until a tool is called. Tools read your
instance's API metadata as they need it: the first card read costs two or three
extra read-only requests, kept for the rest of the session. If the metadata
cannot be read, a built-in copy stands in and pounce tries again on the next call.

### Claude Code

Register the server once for all your projects. `${TP_TOKEN}` is read from your
environment when the server starts, so the token is not stored in the config:

```sh
claude mcp add-json pounce --scope user '{
  "type": "stdio",
  "command": "pounce",
  "env": {
    "TP_BASE_URL": "https://yourcompany.tpondemand.com",
    "TP_TOKEN": "${TP_TOKEN}"
  }
}'
```

Then add the permission rules to `~/.claude/settings.json`:

```json
{
  "permissions": {
    "allow": ["mcp__pounce__read_*"],
    "ask": ["mcp__pounce__write_*", "mcp__pounce__delete_*"],
    "deny": ["mcp__pounce__admin_*"]
  }
}
```

Claude Code names MCP tools `mcp__<server>__<tool>`, so the server name must be
`pounce` for these rules to match. Check the connection with
`claude mcp get pounce`, or `/mcp` inside a session.

### opencode

Add the server and the permission rules to `opencode.json` (global:
`~/.config/opencode/opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "pounce": {
        "type": "local",
        "command": ["pounce"],
        "environment": {
          "TP_BASE_URL": "https://yourcompany.tpondemand.com",
          "TP_TOKEN": "{env:TP_TOKEN}"
        }
      }
    }
  },
  "permissions": [
    { "action": "pounce_read_*", "resource": "*", "effect": "allow" },
    { "action": "pounce_write_*", "resource": "*", "effect": "ask" },
    { "action": "pounce_delete_*", "resource": "*", "effect": "ask" },
    { "action": "pounce_admin_*", "resource": "*", "effect": "deny" }
  ]
}
```

opencode names MCP tools `<server>_<tool>`, so the server name must be `pounce`
for these rules to match (`read_card` becomes `pounce_read_card`). Restart
opencode after changing the file; `opencode mcp list` shows whether it connected.

### Other MCP clients

Any client that starts local (stdio) servers works. Most accept an
`mcpServers` block like this one (Claude Desktop, Cursor, ...):

```json
{
  "mcpServers": {
    "pounce": {
      "command": "pounce",
      "env": {
        "TP_BASE_URL": "https://yourcompany.tpondemand.com",
        "TP_TOKEN": "your-access-token"
      }
    }
  }
}
```

If the client can't find `pounce` (some desktop apps don't see your shell's
`PATH`), give the full path to the file instead, e.g.
`"C:\\Users\\you\\AppData\\Local\\Programs\\pounce\\pounce.exe"` on Windows.
Then map the four tool-name prefixes below to the client's permission system.

## Tools

### Permission tiers

Every tool name starts with its tier, so one rule per tier covers all tools,
including ones added in later versions. Read tools are also marked read-only in
the MCP tool annotations.

| Prefix | What it does | Suggested rule |
|---|---|---|
| `read_` | Reads only | allow |
| `write_` | Creates and updates cards, comments, time, relations, ... | ask |
| `delete_` | Deletes | ask |
| `admin_` | Changes configuration (projects, teams, users, processes, workflows, ...); needs an administrator token | deny |

### Toolsets

58 tools in three toolsets. The client sends every listed tool to the model on
every request, so expose only the toolsets you use: `POUNCE_TOOLSETS=cards`
gives 34 tools, the day-to-day work on cards, for about 4,000 tokens; all 58
cost about 7,500.

**cards** (34): the workflow tools, plus card history and attachments.

- read: `read_card`, `read_search`, `read_my_work`, `read_states`,
  `read_people`, `read_teams`, `read_roles`, `read_projects`, `read_releases`,
  `read_iterations`, `read_custom_field_options`, `read_comments`,
  `read_relations`, `read_times`, `read_attachments`, `read_test_plan`,
  `read_history`
- write: `write_create_card`, `write_update_card`, `write_set_state`,
  `write_assign`, `write_unassign`, `write_set_role_effort`,
  `write_set_custom_fields`, `write_team`, `write_comment`, `write_log_time`,
  `write_relate`, `write_follow`, `write_test_cases`, `write_test_run`,
  `write_attachment`
- delete: `delete_card`, `delete_relation`

**raw** (18): the generic tools, the escape hatch that reaches every resource
and operation your instance reports.

- read: `read_meta`, `read_get`, `read_query`, `read_collection`,
  `read_v2_query`, `read_context`, `read_conversions`, `read_deleted`,
  `read_storage`
- write: `write_create`, `write_update`, `write_bulk`, `write_collection_add`,
  `write_storage`
- delete: `delete_entity`, `delete_bulk`, `delete_collection_remove`,
  `delete_storage`

**admin** (6): `admin_create`, `admin_update`, `admin_delete`,
`admin_collection_add`, `admin_collection_remove`, `admin_undelete`

### Rules the workflow tools enforce

- Effort belongs to a role. `write_set_role_effort` writes role effort rows; the
  card total is computed by Targetprocess and is never written directly.
- Changing a task's role effort makes Targetprocess recalculate its user story;
  the story's before/after values are reported.
- Cards created with `write_create_card` lose the assignments Targetprocess adds
  by default, including ones it adds a moment later, and get exactly the people
  you asked for. What was removed is reported. On existing cards nobody is
  removed unless you ask (`exclusive`).
- State changes and new children report the parent card's state before and after.
- New cards inherit the parent's project. If no project can be resolved, nothing
  is sent.
- A card with child cards is deleted only with `withChildren: true`.
- Names are resolved, never guessed: an ambiguous name returns every candidate
  with its id.
- Time is logged where your process keeps it: standard Time entries, or, on
  instances that track time in a custom `TimeRecord` type (hours and date fields),
  a time record linked to the person and the card.
- Descriptions and comments can be written as Markdown (`format: "markdown"`),
  HTML or plain text, and are read back as Markdown either way.
- Every write is read back, and any value that did not persist is listed under
  `notPersisted`. If a multi-step write fails partway, the error says what was
  already done.
- Attachments are uploaded from base64 content; pounce never reads local files.

## Limitations

- Downloading attachments is not supported. With an access token, Targetprocess
  returns an HTML page instead of the file.
- Automation rules, validation rules, mashups and CSV import have no REST
  management API and are out of scope.

## Development

Building from source, running the tests and releasing are described in
[DEVELOPMENT.md](DEVELOPMENT.md).

## License

[MIT](LICENSE)
