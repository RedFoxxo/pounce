# pounce 🦊

An MCP server for [Targetprocess](https://www.ibm.com/products/targetprocess): lets AI
assistants read and manage your cards through the Targetprocess REST API.

Version 0.1.0. See [CHANGELOG.md](CHANGELOG.md).

## Goals

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

- Node.js 22.12 or newer
- A Targetprocess access token (Settings → Access Tokens in your profile)

## Install

```sh
git clone https://github.com/RedFoxxo/pounce.git
cd pounce
npm ci
npm run build
```

The server entry point is `build/index.js`. It speaks MCP over stdio.

## Configuration

| Variable | Required | Purpose |
|---|---|---|
| `TP_BASE_URL` | yes | Instance root, e.g. `https://yourcompany.tpondemand.com` (no `/api/...`) |
| `TP_TOKEN` | yes | Access token. Redacted from every log line |
| `TP_DEFAULT_PROJECT_ID` | no | Fallback project, used only when a new card has no parent to inherit one from |
| `TP_DEFAULT_TEAM_ID` | no | Fallback team for new cards; omitted when unset |

pounce reads these from its environment and exits with a clear message if a
required one is missing. It does not read `.env` files; `.env.example` lists
the variables for reference.

## opencode setup

Add the server and the four permission rules to `opencode.json` (global:
`~/.config/opencode/opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "pounce": {
      "type": "local",
      "command": ["node", "/path/to/pounce/build/index.js"],
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

opencode prefixes MCP tools with the server's config key, so the key must be
`pounce` for the rules above to match (`read_card` is exposed as
`pounce_read_card`). Tools are registered without a prefix of their own, and
every new tool falls under one of the four rules automatically.

## Claude Code setup

Register the server once for all your projects. `${TP_TOKEN}` is expanded from
your environment when the server starts, so the token is not stored in the
config file:

```sh
claude mcp add-json pounce --scope user '{
  "type": "stdio",
  "command": "node",
  "args": ["/path/to/pounce/build/index.js"],
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
`pounce` for these rules to match. Check the connection with `claude mcp get
pounce` or `/mcp` inside a session.

## Permission tiers

`admin_*` tools write configuration (projects, teams, users, processes,
workflows, ...) and need an administrator token. Keep them denied unless you mean
to use them.

Any other MCP client works the same way: start `node build/index.js` with the
environment variables above, and map the four tool-name prefixes to its
permission system.

## Tools

58 tools in four tiers. Workflow tools (layer 2) are the ones to use day to day.
Generic tools (layer 1) are the escape hatch that reaches every resource and
operation your instance reports.

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
- A task's role efforts roll up into its user story, replacing the story's value
  for that role. The story's before/after values are reported.
- Cards created with `write_create_card` lose the assignments Targetprocess adds
  by default, and get exactly the people you asked for. What was removed is
  reported. On existing cards nobody is removed unless you ask (`exclusive`).
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

```sh
npm test            # unit + contract tests (stubbed fetch, no network)
npm run typecheck   # src and tests
npm run build       # compile to build/
npm run snapshot    # regenerate src/catalog/snapshot.json from TP_BASE_URL
```

`CLAUDE.md` holds the architecture, domain rules, conventions and the confirmed
Targetprocess API surface, including the quirks found on a live instance.

### Live tests

Live tests hit the instance in `TP_BASE_URL`, run only with `TP_LIVE=1`, and are
never part of `npm test`. The read-only ones need nothing else:

```sh
npm run test:live
```

The acceptance run **writes**. It changes the state, assignments, role efforts
and BackEnd/FrontEnd fields of a user story you name, and creates a task and a
bug under it (both deleted afterwards unless `TP_LIVE_KEEP=1`):

```sh
TP_LIVE_STORY=<test story id> \
TP_LIVE_DEVELOPERS="<person>,<person>" \
TP_LIVE_PRODUCT_OWNER="<person>" \
npm run test:live -- tests/live/acceptance.test.ts
```

Optional: `TP_LIVE_BUG_ASSIGNEE` (defaults to the first developer) and
`TP_LIVE_TEAM` (defaults to `Core Team`).
