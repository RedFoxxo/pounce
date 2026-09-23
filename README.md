# pounce 🦊

An MCP server for [Targetprocess](https://www.ibm.com/products/targetprocess): lets AI
assistants read and manage your cards through the Targetprocess REST API.

Version 1.0.0. See [CHANGELOG.md](CHANGELOG.md).

## Why pounce

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

- Node.js 22.12 or newer (`npx` comes with it)
- A Targetprocess access token: in Targetprocess, open your profile →
  **Access Tokens** → create one

Nothing to download or build: your MCP client starts pounce with `npx`, which
fetches the package from npm on first use.

## Configuration

pounce reads its settings from environment variables, which your MCP client
passes to it:

| Variable | Required | Purpose |
|---|---|---|
| `TP_BASE_URL` | yes | Your instance, e.g. `https://yourcompany.tpondemand.com` (no `/api/...`) |
| `TP_TOKEN` | yes | Your access token. Never logged |
| `TP_DEFAULT_PROJECT_ID` | no | Project for new cards that have no parent to inherit one from |
| `TP_DEFAULT_TEAM_ID` | no | Team for new cards when none is given |

Keep the token out of config files: export it in your shell profile
(`export TP_TOKEN=...`) and let the client pass it through, as the examples
below do.

## opencode

Add the server and the four permission rules to `opencode.json` (global:
`~/.config/opencode/opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "pounce": {
      "type": "local",
      "command": ["npx", "-y", "pounce@1"],
      "enabled": true,
      "environment": {
        "TP_BASE_URL": "https://yourcompany.tpondemand.com",
        "TP_TOKEN": "{env:TP_TOKEN}"
      }
    }
  },
  "permission": {
    "pounce_read_*": "allow",
    "pounce_write_*": "ask",
    "pounce_delete_*": "ask",
    "pounce_admin_*": "deny"
  }
}
```

opencode prefixes MCP tools with the server's key, so the key must be `pounce`
for these rules to match (`read_card` becomes `pounce_read_card`). Restart
opencode after changing the file; `opencode mcp list` shows whether it connected.

## Claude Code

Register the server once for all your projects. `${TP_TOKEN}` is read from your
environment when the server starts, so the token is not stored in the config:

```sh
claude mcp add-json pounce --scope user '{
  "type": "stdio",
  "command": "npx",
  "args": ["-y", "pounce@1"],
  "env": {
    "TP_BASE_URL": "https://yourcompany.tpondemand.com",
    "TP_TOKEN": "${TP_TOKEN}"
  }
}'
```

Then add the four permission rules to `~/.claude/settings.json`:

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

## Other MCP clients

Any client that starts local (stdio) servers works. Most accept an
`mcpServers` block like this one (Claude Desktop, Cursor, ...):

```json
{
  "mcpServers": {
    "pounce": {
      "command": "npx",
      "args": ["-y", "pounce@1"],
      "env": {
        "TP_BASE_URL": "https://yourcompany.tpondemand.com",
        "TP_TOKEN": "your-access-token"
      }
    }
  }
}
```

Then map the four tool-name prefixes below to the client's permission system.

## Permission tiers

Every tool name starts with its tier, so one rule per tier covers all tools,
including ones added in later versions:

| Prefix | What it does | Suggested rule |
|---|---|---|
| `read_` | Reads only | allow |
| `write_` | Creates and updates cards, comments, time, relations, ... | ask |
| `delete_` | Deletes | ask |
| `admin_` | Changes configuration (projects, teams, users, processes, workflows, ...); needs an administrator token | deny |

## Tools

58 tools. Workflow tools are the ones to use day to day. Generic tools are the
escape hatch that reaches every resource and operation your instance reports.

**read** (26)

- Workflow: `read_card`, `read_search`, `read_my_work`, `read_states`,
  `read_people`, `read_teams`, `read_roles`, `read_projects`, `read_releases`,
  `read_iterations`, `read_custom_field_options`, `read_comments`,
  `read_relations`, `read_times`, `read_attachments`, `read_test_plan`
- Generic: `read_meta`, `read_get`, `read_query`, `read_collection`,
  `read_v2_query`, `read_history`, `read_context`, `read_conversions`,
  `read_deleted`, `read_storage`

**write** (20)

- Workflow: `write_create_card`, `write_update_card`, `write_set_state`,
  `write_assign`, `write_unassign`, `write_set_role_effort`,
  `write_set_custom_fields`, `write_team`, `write_comment`, `write_log_time`,
  `write_relate`, `write_follow`, `write_test_cases`, `write_test_run`
- Generic: `write_create`, `write_update`, `write_bulk`,
  `write_collection_add`, `write_storage`, `write_attachment`

**delete** (6): `delete_card`, `delete_relation`, `delete_entity`, `delete_bulk`,
`delete_collection_remove`, `delete_storage`

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
- Names are resolved, never guessed: an ambiguous name returns every candidate
  with its id.
- Time is logged where your process keeps it: standard Time entries, or, on
  instances that track time in a custom `TimeRecord` type (hours and date fields),
  a time record linked to the person and the card.
- Every write is read back, and any value that did not persist is listed under
  `notPersisted`. If a multi-step write fails partway, the error says what was
  already done.

### Known limitations

- Downloading attachments is not supported. With an access token, Targetprocess
  returns an HTML page instead of the file.
- Automation rules, validation rules, mashups and CSV import have no REST
  management API and are out of scope.

## Development

Building from source, running the tests and releasing are described in
[DEVELOPMENT.md](DEVELOPMENT.md).
