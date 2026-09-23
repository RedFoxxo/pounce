# CLAUDE.md

Guidance for working in this repository.

## What this is

**pounce** is an MCP server for the [Targetprocess](https://www.ibm.com/products/targetprocess)
REST API, built from scratch to replace an older fork
(`SerhiiMaksymiv/targetprocess-mcp-server`), which has been retired.

Goal: **full coverage.** If Targetprocess exposes an operation, pounce can
perform it. Coverage is achieved in two layers (see "Architecture"), not by
registering one tool per endpoint.

Status: version 1.0.1 (58 tools), verified end to end against the live
instance. Published to npm; users install it with `npx`, not from source. Build,
test and release steps are in `DEVELOPMENT.md`. See "Status and remaining work"
at the end.

## Why a rewrite — lessons from the old server

Every rule below exists because the old server broke on it. Do not reintroduce
any of these.

| Failure in the old server | Rule in pounce |
|---|---|
| `get_processes` read v2 `items` from a v1 `Items` payload and always said "No processes found" | One HTTP layer per API version, each with typed response shapes. Never hand-read `items`/`Items` in a handler. |
| Workflow lookups filtered on a hardcoded process id `89` from someone else's instance | **No hardcoded ids, anywhere.** Resolve projects/processes/workflows from the card or project the call is about. |
| `ownerId` defaulted to a hardcoded `"1504"` | Same rule. Identify the current user via `Context` / `Users/LoggedUser`. |
| Creators posted `Project: { Id: "" }` and `assignedTeams: [{ team: { id: "" } }]` when env vars were unset → opaque 400 | Never send an empty reference. Inherit the project from the parent card; fail **before** posting with an actionable message if nothing resolves. |
| `post()` threw away the response body, so errors were a bare "status: 400" | Every request returns a result carrying `status` and TP's raw error `body`. Every failing tool sets `isError: true`. |
| `get_users` sent no `take`/`skip` and silently stopped at TP's default page of 25 | **Every list is fully paged.** A capped list must say so (`truncated: true`). |
| `effort` wrote the card total, which TP computes from per-role efforts | Effort is written to `RoleEffort`, never to the card's `Effort` field. |
| No way to list who is assigned to a card; TP silently assigned a default Product Owner on creation | A card read always includes its assignments. Creating a card clears TP's default assignments. |
| `instanceof Error` compiled against a result union but was always false at runtime | No `instanceof Error` on API results. Use a discriminated `Result` and narrow on `ok`. |
| Mutation tools had no shared name pattern, so the opencode allowlist had to enumerate each one and new tools were silently filtered out | Tools are grouped by tier prefix (`read_`, `write_`, `delete_`, `admin_`). One permission glob per tier. |
| The domain rules lived in the user's local agent file, so a colleague cloning the repo didn't get them | Domain rules are enforced **in the server** and stated in tool descriptions. |
| Tests mocked the shape the handler expected instead of the shape the client returned | Contract tests drive the real client through the handler with a stubbed `fetch`. |

## Stack

- TypeScript (strict), ESM, Node `>=22.12` (Node 20 is end-of-life, and vitest 5
  requires 22.12)
- `@modelcontextprotocol/sdk` with the stdio transport
- `zod` for tool input schemas
- `vitest` for tests
- No other runtime dependencies. HTML ↔ text for descriptions is a small
  in-house converter (`src/format/html.ts`); add a library only for a concrete
  need it cannot meet.

Versions are pinned exactly. Before every commit run `npm run typecheck`,
`npm test` and `npm run build`; `npm run snapshot` regenerates
`src/catalog/snapshot.json` from `TP_BASE_URL` (read-only).

## Configuration

| Variable | Required | Purpose |
|---|---|---|
| `TP_BASE_URL` | yes | Instance root, e.g. `https://mamami.tpondemand.com` (no `/api/...` suffix) |
| `TP_TOKEN` | yes | Access token. Sent as `access_token`; **redact it from every log line** |
| `TP_DEFAULT_PROJECT_ID` | no | Fallback only when a call has no card/parent to inherit a project from |
| `TP_DEFAULT_TEAM_ID` | no | Fallback team for creations; omitted from payloads when unset |

That is the complete list. Anything else must be resolved from the API.
Fail at startup with a clear message if a required variable is missing.

## Architecture

```
src/
  index.ts             bootstrap: config, context, stdio transport
  server.ts            tool registration; tier check and MCP annotations
  config.ts            env loading + validation
  log.ts, version.ts
  http/
    core.ts            the one request function: token, Accept, redacted stderr log, Result
    v1.ts              REST v1: get / list (auto-paging) / create / update / bulk / delete / collection remove
    v2.ts              REST v2 and history v2 queries (read-only, auto-paging)
    result.ts          Result<T> = { ok: true, status, data } | { ok: false, status, message, body }
    redact.ts          token redaction
  catalog/
    loader.ts          index scan, JSON/XML /meta, history probing
    catalog.ts         lookups, suggestions, live-with-snapshot-fallback provider
    snapshot.json      committed baseline catalog (regenerate with npm run snapshot)
    snapshot-cli.ts
  resolve/             name → id: directory (users, roles, teams, projects, states,
                       custom fields; cached), match (never-guess rules), card (id → type/project)
  domain/              the Targetprocess rules: cards (read_card include/shape, parent
                       chain), parent (side-effect snapshots), assignments, efforts,
                       custom-fields, create (parent links)
  format/              dates, HTML ↔ text, response shaping
  tools/
    types.ts, context.ts, schema.ts, respond.ts, index.ts
    generic/           layer 1: read, write (+ admin), extras (v2, history, storage,
                       attachments, undelete), validate (catalog payload checks), policy (admin gating)
    workflow/          layer 2: read, write, delete, common
scripts/copy-assets.mjs  copies snapshot.json into build/
tests/                 contract tests (tools/), helpers (fetch stub, MCP harness, fake
                       Targetprocess), fixtures, live/ (TP_LIVE=1 only)
```

### HTTP layer

- A single request function per API version. All tool code goes through it.
- Returns `Result<T>`, never throws for HTTP errors, never returns a bare `Error`.
- `list()` pages with `take`/`skip` until `Next` is absent. TP's default page is
  25 and its maximum is 1000 (a larger `take` is silently treated as 1000), so
  page with `take=1000`. Inner collections in `include` are capped separately:
  pass `innerTake`. A hard total cap (e.g. 5000) is reported as
  `truncated: true`, never silently.
- JSON in and out (`format=json`). v1 dates arrive as `/Date(ms+offset)/` —
  convert to ISO 8601 in `format/`, **keeping the offset** (`…+02:00`): release,
  iteration and time dates are stored as local midnight (**live**), and a UTC
  rendering would show the previous day. v2 accepts `isoDate`, uses camelCase, and
  **omits null fields** — absence means null.
- On writes, shape the response with `resultFormat=json` and `resultInclude`, so
  the read-back needs no extra request when the response is enough.
- v1 and v2 filter syntax differ (`eq` vs `==`/`=`); v1 `where` does not support
  `or`. Never share filter strings between the two clients.
- v1 sorts with `orderBy=Field` or `orderByDesc=Field`; `orderBy=Field desc` is a
  400 "Error during parameters parsing" (**live**).
- Send **both** `format=json` and `Accept: application/json` (**live**). With only
  `format=json`, errors come back as XML; with only the header, `/meta` comes
  back as XML. With both, errors are JSON `{Status, Message, Details, ErrorId}`.
  Summaries use `Message`; the raw body is always kept.
- **curl note for manual probing:** pass `-g`. `[...]` in `include` and `{...}` in
  `select` are curl glob patterns and silently mangle the request otherwise.
- Log method + redacted URL to stderr. stdout belongs to the MCP transport.

### Layer 1 — generic, catalog-driven tools (full coverage)

The catalog is read from the user's own instance at startup
(`/api/v1/Index/meta`, then `/api/v1/{Resource}/meta` for each resource), cached
in memory, and falls back to `snapshot.json` if metadata is unavailable. Each
entry records the resource name, plural path, `CanCreate` / `CanUpdate` /
`CanDelete`, every field with `CanSet` / `IsRequired` / type, and every
collection with `CanAdd` / `CanRemove`.

Four confirmed quirks the loader must handle:

- **The index is not usable as plain JSON.** `/api/v1/Index/meta` repeats the
  key `"ResourceMetadataDescription"` once per resource, so `JSON.parse` keeps
  only the last one. Scan the raw text for every occurrence instead.
- **The index is incomplete.** History resources and `GeneralConversions` answer
  `/meta` but are not listed (56 on our instance). For every resource `X`, also
  probe `XHistories/meta` and `XSimpleHistories/meta`, and always probe
  `GeneralConversions/meta`.
- **An index entry can be broken.** `SickLeave` is listed on our instance but its
  `/meta` and its collection both return 404. Record such entries as
  unavailable and keep loading; never fail startup over one resource.
- **Some `/meta` is XML only.** `Context/meta` answers XML even with
  `format=json` and `Accept: application/json` (**live**); the loader parses both.

Generic tools validate every call against the catalog **before** sending it:
unknown resource, unsupported operation, unknown field, or non-settable field is
rejected with a message listing the valid options.

| Tool | Operation |
|---|---|
| `read_meta` | Describe a resource: operations, fields, collections |
| `read_get` | GET one entity by resource + id, with `include` |
| `read_query` | GET a collection with `where`, `include`, `orderBy`, fully paged |
| `read_collection` | GET a sub-collection, e.g. `UserStories/{id}/Tasks` |
| `read_v2_query` | v2 query (`select`, `where`, aggregations) |
| `read_history` | Simple or full change history of any entity; the resource is resolved from the id when omitted, so it also serves cards (see "Confirmed API surface") |
| `read_context` | `Context` for given entity/project/team ids: processes, practices, terms, custom field definitions |
| `read_conversions` | `GeneralConversions`: an entity's id before or after a type conversion |
| `read_deleted` | Deleted projects/users via v2 `includeDeleted=true` |
| `read_storage` | RESTful storage groups, storages, and queries (views/boards live here) |
| `write_create` | POST a new entity (resources with `CanCreate`), optionally with nested children |
| `write_update` | POST an update to an existing entity (`CanUpdate`) |
| `write_bulk` | Create or update up to 500 entities of one resource in one call |
| `write_collection_add` | Add an item to a collection with `CanAdd` |
| `write_storage` | Create/merge a storage entry (merge semantics, see below) |
| `write_attachment` | Upload files to an entity via `UploadFile.ashx` |
| `delete_entity` | DELETE an entity (`CanDelete`) |
| `delete_bulk` | Delete up to 500 entities of one resource by id |
| `delete_collection_remove` | Remove items from a collection with `CanRemove` |
| `delete_storage` | Delete a storage entry |
| `admin_create` | POST a new configuration/administration entity (list below) |
| `admin_update` | Update a configuration/administration entity |
| `admin_delete` | Delete a configuration/administration entity |
| `admin_collection_add`, `admin_collection_remove` | Collection add/remove on a configuration/administration entity (e.g. Team `TeamMembers`) |
| `admin_undelete` | Restore deleted entities (administrator token required) |

Configuration and administration resources go through `admin_create`,
`admin_update`, `admin_delete` instead, and the generic write/delete tools
(including bulk and collection tools) refuse them: `CustomRule`, `CustomField`, `EntityPermission`, `EntityState`,
`GlobalSettings`, `Priority`, `Process`, `Program`, `Project`, `ProjectMember`,
`RequestType`, `Role`, `RoleEntityType`, `RoleEntityTypeProcessSetting`,
`Severity`, `Team`, `TeamMember`, `TeamProject`, `Term`, `User`, `Workflow`.
Review this list against real usage rather than treating it as final.

Layer 1 is how "no missing features" is guaranteed: any resource the instance
reports is reachable, including ones added by a future TP version.

### Layer 2 — curated workflow tools

These encode how the team actually works, accept names instead of ids, and apply
the domain rules. Prefer them; layer 1 is the escape hatch.

**Read**

| Tool | Notes |
|---|---|
| `read_card` | Any card by id: type, name, state, project, parent chain, teams with team states, **assignments**, **role efforts**, effort totals, release/iteration, custom fields, tags, counts. Description as plain text. |
| `read_search` | Cards by text, type, state, project, assignee, tag |
| `read_my_work` | Cards assigned to the current user, by state |
| `read_states` | States available to a card (or project + entity type), including team sub-workflow states flagged `isTeamWorkflow` |
| `read_people` | Users by name/login/email, fully paged; ambiguous matches listed, never guessed |
| `read_teams`, `read_roles`, `read_projects`, `read_releases`, `read_iterations` | Reference data |
| `read_custom_field_options` | Custom fields of a card's type (or project + type) with their allowed dropdown values |
| `read_comments`, `read_relations`, `read_times`, `read_attachments` | Per card (history: layer 1 `read_history`) |
| `read_test_plan` | Test plan with its test cases and steps; test runs |

**Write**

| Tool | Notes |
|---|---|
| `write_create_card` | Any card type with parent, title, description, state, teams, assignees, role efforts, tags, custom fields. Applies the creation rules below. |
| `write_update_card` | Title, description, tags (set or add/remove), release, iteration, team iteration, parent, other settable fields |
| `write_set_state` | By state name or id; resolves against the card's own workflow; reports parent side effects |
| `write_assign` | User + role by name or id; `exclusive: true` removes others in that role only when explicitly asked |
| `write_unassign` | One exact assignment |
| `write_set_role_effort` | One or more roles on a card; reports rollup side effects |
| `write_set_custom_fields` | Validates dropdown values against `read_custom_field_options` before writing |
| `write_team` | Add or remove a team on a card |
| `write_comment`, `write_relate`, `write_follow` | Attachments: layer 1 `write_attachment` |
| `write_log_time`, `read_times` | `Time` entries where the card's process has the "Time Tracking" practice; otherwise a `TimeRecord` type when the instance has one with an "Hours" field (no role, no remaining time); otherwise a clear error |
| `write_test_cases` | Create test cases with steps under a test plan |
| `write_test_run` | Record a test plan run and per-test-case results |

The `delete_` tier adds only what layer 1 `delete_entity` cannot express:
`delete_card` (card by id alone, type resolved, parent side effects reported;
a card with child cards needs `withChildren: true`; what counts as a child
depends on the type: a story's tasks and bugs, a feature's stories, an epic's
features, a test plan's test cases, but never a test case's linked stories) and `delete_relation` (by the
two related card ids). Comments, times and other plain entities are deleted with
`delete_entity`.

The `fields` escape hatch of `write_create_card` / `write_update_card` refuses
fields a rule applies to (`Effort`, `Assignments`, `AssignedTeams`,
`RoleEfforts`, `CustomFields`, `EntityState`, `Project`, parent references,
tags, ...) and names the dedicated argument or tool instead. `write_attachment`
takes base64 content only; pounce never reads local files for upload.

## Domain rules (enforced in code)

1. **Effort belongs to a role.** A card's `Effort` is the total TP computes from
   its `RoleEffort` rows. `write_set_role_effort` writes the rows. Nothing writes
   the card `Effort` field except `write_update` on explicit request. If no role
   is named, the tool errors and asks for one.
2. **Task efforts roll up.** TP recomputes the parent story's role efforts when
   a task's role effort changes (**live**: story Developer 8 → 7 → 5 as its task
   went 1h → 3h, story total unchanged). Writing a task's effort must report the
   parent's before/after values rather than claiming the story estimate was
   kept. Totals are recomputed a few seconds after a write (**live**), so totals
   read right after it can lag; say so.
3. **Default assignments are cleared on creation.** After creating a card, read
   its assignments, remove those TP added by default, then add the requested
   people. TP can add a default **after** the first check (**live**: a bug got its
   default Product Owner a moment later), so look again after a short pause
   (`settleDelaysMs`) and remove late ones too (`late: true`). Report what was
   removed. On pre-existing cards, never remove anyone unless explicitly asked.
4. **Parent state can move.** Moving a child task out of its initial state can
   advance the parent story. `write_set_state` and `write_create_card` read the
   parent's state before and after, and report any change.
5. **Projects are inherited.** Task ← user story, story ← feature, feature ←
   epic, bug ← the card it is raised from, test case ← test plan, test plan ← the
   card it covers. Fallback to `TP_DEFAULT_PROJECT_ID`; otherwise fail before
   posting.
6. **Resolve names; never guess.** Users, roles, teams, states, projects and
   custom field options accept names. Zero matches → error with suggestions.
   Several matches → error listing all candidates with ids. An exact name wins
   over partial matches; an exact hit on an inactive person or team is refused
   for new assignments but accepted for reads and removals.
7. **Verify writes.** Every workflow write reads the entity back and reports any
   requested value that did not persist.
8. **Writes to one card are sequential.** TP recomputes derived fields on every
   write; parallel writes to one card can lose updates.
9. **Collection writes append.** Posting `Assignments`, `AssignedTeams` or
   `TagObjects` adds to what is there. Anything that means "set" or "only" must
   delete the existing items first, and say what it removed.
10. **Say what is known.** If a read-back or a parent read fails, report a
    warning (or `unknown`) instead of a verdict. If a multi-step write fails
    partway, list what was already done. A write with no response (timeout) has
    an unknown outcome: say so rather than implying nothing happened.

## Tool conventions

- Register tools **without** a `pounce_` prefix. opencode prefixes tools with the
  server's config key (`pounce`), so `read_card` is exposed as
  `pounce_read_card`. A built-in prefix would produce `pounce_pounce_read_card`.
- Tier prefixes are the permission contract:
  ```yaml
  pounce_read_*: allow
  pounce_write_*: ask
  pounce_delete_*: ask
  pounce_admin_*: deny
  ```
  The current token user is not an administrator, so `admin_` tools will get
  403 from Targetprocess regardless.
  Never put a mutation in `read_`.
- Every id parameter is validated as `^\d+$`.
- Tool descriptions state the domain rule that applies, e.g. `write_set_role_effort`
  explains the rollup.
- Responses are compact JSON with ids and names. HTML descriptions are returned
  as text. Errors: `isError: true`, a one-line summary, then `status` and `body`.

## Testing

- **Contract tests** (`tests/tools/`) stub `fetch` with the payload TP really
  returns and drive the real client through the MCP server
  (`tests/helpers/harness.ts`: in-memory transport, real handlers). Required for
  every tool.
- **Stateful fake** (`tests/helpers/fake-tp.ts`) reproduces the behaviours the
  domain rules exist for: default assignments on creation, task effort rollup,
  a story moving when a task leaves its initial state. Write-tool and
  acceptance tests run against it.
- **Payload tests** assert the exact request body: no empty references, no
  unrequested fields.
- **Catalog coverage test**: every resource × operation and every addable/removable
  collection in `snapshot.json` is reachable through a layer 1 tool.
- **Live tests** (`npm run test:live`, `tests/live/`) run only with `TP_LIVE=1`
  and are never part of `npm test`. Read tests need nothing else; the acceptance
  run writes and needs a story the user names (`TP_LIVE_STORY`, see README).
- When fixing a bug, first write a test that fails against the old code.

## Confirmed API surface

Researched on 2026-09-23 from the official docs (IBM Docs, Targetprocess Developer
Hub: `https://www.ibm.com/docs/en/SSNFIB/targetprocess/dev-hub/<page>.html`) and
verified against our instance `https://mamami.tpondemand.com`, Targetprocess
**2609.2.0.3492**, Pro edition. Live checks were read-only GETs, plus one no-op
bulk POST with an empty array (created nothing). The token user (2286) is **not an
administrator**.

Evidence key: **live** = verified on our instance; **docs** = documented, not
exercised live because it writes data. Verify every **docs** row against a test
card (the live acceptance run) before relying on it.

### Resources

Our instance exposes **89 resources** in `/api/v1/Index/meta` (78 full CRUD, 3
update + delete, 2 update only, 5 read only, 1 broken), plus **56 unlisted
resources** found by probing every `{X}Histories`/`{X}SimpleHistories` and
`GeneralConversions`, and **1054 collections**, 543 of them addable/removable.
`src/catalog/snapshot.json` holds the full per-resource catalog (fields,
references, collections with their flags); `read_meta` shows it, and
`tests/catalog.test.ts` pins these counts.

Compared with the public demo instance (`md5.tpondemand.com`, 69 resources),
ours has 21 extra **Extendable Domain** types (AcceptanceCriterion, ActionItem,
ActionItemFeedback, DayPeriod, Feedback, IterationGoal, Location, MonthPeriod,
Overtime, PublicHoliday, PublicHolidayLocation, QuarterPeriod, Rate,
Retrospective, SickLeave, TimeRecord, Timesheet, UserRate, Vacation, WeekPeriod,
YearPeriod) and lacks `AgileReleaseTrain`. This is why the catalog must be read
from the instance, never hardcoded.

### Operations

| Capability | How | Evidence |
|---|---|---|
| Auth | `access_token=<PAT>` query parameter. Alternatives: `apptio-opentoken` header, service `token`, Basic. Never call `GET /api/v1/Authentication` — it returns a service token | docs; PAT **live** |
| Current user | `GET /api/v1/Users/LoggedUser` | **live** |
| Where literals | Strings in v1 `where` are escaped with a backslash: `'it\'s'`; `''` is a 400 | **live** |
| Tag filter | `TagObjects.Name eq 'x'` matches a tag exactly (case-insensitive) | **live** |
| Assignee filter | `AssignedUser.Id eq N` on Assignables; `Assignments.GeneralUser.Id eq N` does not filter reliably | **live** |
| Context | `GET /api/v1/Context` with `?ids=`, `?projectIds=&teamIds=`, `?acid=`: processes, practices (e.g. `IsStoryEffortEqualsSumTasksEffort`), terms, custom field definitions. No partial get. Read-only in practice despite its meta | **live** + docs |
| Read one / list | `GET /api/v1/{Plural}/{id}`, `GET /api/v1/{Plural}` with `where`, `include`, `exclude`, `append` (e.g. `[Tasks-Count]`), `orderBy`, `take` ≤ 1000, `skip`, `innerTake` | **live** |
| Inner collection | `GET /api/v1/{Plural}/{id}/{Collection}`. You cannot POST into an inner collection URL; create the child on its own collection with a reference to the parent | docs |
| v2 query | `GET /api/v2/{Entity}` (singular) with `select`, `where`, `result` (aggregations), `orderBy`, `take`, `skip`, `filter` (board DSL), `isoDate`. Read-only. Without `isoDate=true` dates are `/Date(...)/` | **live** + docs |
| Create | `POST /api/v1/{Plural}` without `Id` → 201 | docs; used by old server |
| Update | `POST /api/v1/{Plural}` or `/{Plural}/{id}` **with** `Id` → 200. Posting an `Id` to create performs an update | docs |
| Nested create | e.g. story with `"Tasks":{"Items":[...]}` in one POST | docs |
| Set collections | `"Assignments"`, `"AssignedTeams"`, `"RoleEfforts"` as `{"Items":[...]}` on the card POST. **POST appends; to replace, DELETE the existing items first** | docs |
| Tags | `"Tags":"a,b"` replaces all tags; `"TagObjects":[{"Name":...}]` or `[{"Id":...}]` adds | docs |
| Rich text | Descriptions and comments are HTML by default; Markdown is stored as text starting with `<!--markdown-->` (**live**: stored byte for byte; the instance has Markdown descriptions and comments written in the UI). `textToHtml(text, format)` adds the marker for `format: "markdown"`; `htmlToText` reads HTML back as light Markdown and returns Markdown unchanged | **live** |
| Custom fields | By name (`"MyField": value`) or `"CustomFields":[{"Name","Value"}]`; entity-type fields take `{"Id","Kind"}`. System fields (`IsSystem`, e.g. "Total Hours") are refused with a 400 that fails the **whole** write, so pounce refuses them before sending | `CustomFields` array **live**; entity form docs |
| Time tracking | `POST /api/v1/Times` answers 400 "Time is not available for current process" where the process lacks the "Time Tracking" practice (`Processes/{id}?include=[Practices[Name]]`; **live**, process 13). Our instance tracks time in a custom `TimeRecord` type instead: custom fields "Hours" (number, required) and "Date", references `ConnectedUser` and `Task`/`UserStory`/`Bug`/…; an automation renames records ("<card> / <person> / <hours>h") and fills day/week/month periods. `CustomFields.Date gte '…'` filters server-side. `write_log_time`/`read_times` pick `Time` or `TimeRecord` per process (`src/domain/time.ts`) | **live** |
| Response shaping on write | `resultFormat`, `resultInclude`, `resultExclude`, `resultAppend` | docs |
| Bulk create/update | `POST /api/v1/{Plural}/bulk` with an array, **≤ 500 items** | route **live** (400 on `[]`), docs |
| Delete | `DELETE /api/v1/{Plural}/{id}` → 200, 404 if missing | docs; used by old server |
| Bulk delete | `DELETE /api/v1/{Plural}/bulk` with `[{"Id":..}]`; by id only | docs |
| Remove from collection | `DELETE /api/v1/{Plural}/{id}/{Collection}/{childId}`, one per child. The documented `?childrenIds=1,2` form answers **500** (**live**) | **live** |
| Upload attachment | `POST /UploadFile.ashx`, multipart, fields `generalId` + one or more `file`; works with an access token | **live** |
| Download attachment | `GET /Attachment.aspx?AttachmentID={id}` needs Basic or cookie auth. **With a PAT it returns an HTML error page, not the file** | **live** (limitation confirmed) |
| History (simple) | `/api/v1/{Plural}/{id}/History`, `{Entity}SimpleHistories` (v1 and v2). Only records state/effort/release/iteration changes | **live** |
| History (full) | `{Entity}Histories` (v1/v2) and `/api/history/v2/{Entity}`, with `IsChanged{Field}` flags and a `Changes` list (**only returned when explicitly included**; use the flags); also for Extendable Domain types. Filter by `SourceEntityId` (v1) / `sourceEntityId` (history v2); there is no `UserStory` reference on `UserStoryHistory` | **live** |
| Conversions | `GET /api/v1/GeneralConversions` (`FromGeneralID`, `ActualGeneral`) | **live** |
| Deleted items | `GET /api/v2/projects` or `/users` with `where=(DeleteDate!=null)&includeDeleted=true`. The v2 `next` link **drops** `includeDeleted`: page with your own `skip` and repeat every parameter | **live** |
| Undelete | `POST /api/v1/undelete` `{"Id","EntityType"}` and `/api/v1/undelete/bulk`. **Administrator token required** — ours is not. Comments, milestones and programs cannot be undeleted | docs |
| RESTful storage | `/storage/v1/` groups, `/storage/v1/{Group}` (query with `select`/`where`/`take`/`skip`), `/storage/v1/{Group}/{Key}` GET/POST/DELETE. POST **merges** `publicData`/`userData` (a `null` value deletes a key). Views and boards are stored here (`boards`, `boardGroups`, ...) | groups, group query and entry GET **live**; POST/DELETE docs |
| Direct access permissions | `EntityPermission` CRUD + `/bulk`; feature is off by default | docs |

### Not in scope, with reasons

- **Automation rules, validation rules, mashups, email templates, CSV import**:
  configured in the Targetprocess UI; no documented REST management API.
  Incoming webhooks are per-rule URLs, not a Targetprocess API.
- **WorkSharing API**: only available inside automation-rule JavaScript
  (`context.getService('workSharing/v2')`), not to external REST clients.
- **`/api/login/validate`, `/api/login/ValidateRequester`**: unauthenticated
  username/password checks. They serve no MCP use case and would turn pounce into
  a credential-testing tool. Deliberately excluded.
- **Attachment download**: blocked with PAT auth (above). Add it only if a Basic-auth
  or cookie option is introduced deliberately.

Write shapes inferred from the metadata rather than shown in the docs, and
their live status (test story #36512, 2026-09-23):

- **live:** `POST /api/v1/RoleEfforts/{id}` `{Effort}` (row update);
  `POST /api/v1/TeamAssignments/{id}` `{EntityState}` and
  `DELETE /api/v1/TeamAssignments/{id}`; `POST /api/v1/GeneralFollowers`
  `{General, User}` and `DELETE /api/v1/GeneralFollowers/{id}`;
  `POST /api/v1/Assignments` and `DELETE /api/v1/Assignments/{id}`;
  `"Tags": ""` clearing every tag; `POST /api/v1/{Plural}/bulk` updates; test plan runs
  and `TestCaseRuns` `{Status, Comment}`; `UploadFile.ashx` with an access token.
- **not yet exercised:** `POST /api/v1/RoleEfforts` `{Assignable, Role, Effort}`
  (creating a missing row; cards came with a row per role); `Time.Date` as
  `YYYY-MM-DD` (time tracking is off in our process; `TimeRecord` is used and
  verified live instead); whether deleting a user
  story also deletes its tasks: **yes** (**live**, #36531/#36532 deleted in the
  same millisecond), which is why `delete_card` requires `withChildren: true`.

Unresolved: the docs also show `/api/deletedItems/v1/{projects|users}/{id}/restore`,
but `GET /api/deletedItems/v1/projects` returns 404 on our instance. Prefer
`/api/v1/undelete` and verify either route with an administrator before use.

## Status and remaining work

Done: scaffold, HTTP layer and catalog, layer 1 tools with admin gating,
resolvers, layer 2 read/write/delete tools, v2/history/storage/conversions/
deleted items/attachment upload, README and opencode setup. Independent reviews
of the write paths (twice), the read paths and the infrastructure; their
confirmed findings are fixed with regression tests.

Remaining:

1. **Live acceptance run.** Done interactively on test story #36512 under
   feature #36193 (2026-09-23): every workflow tool that can act on a story and
   its children, plus the generic tools, ran against the live instance; the bugs
   it found are fixed and re-verified live. The scripted twin
   (`tests/live/acceptance.test.ts`) runs against a user story the user names. It reproduces the #36410 batch in one pass: state Ready, two
   Developers + a Product Owner, role efforts, clear BackEnd / set FrontEnd, a
   task in Coded with Core Team, 1h Developer effort and one assignee, a bug
   assigned to one person, default assignments removed, parent state reported.
   It already passes against the fake (`tests/tools/acceptance.test.ts`). Card
   #36410 itself no longer exists (404). While running it, confirm the
   metadata-inferred write shapes listed under "Confirmed API surface".
2. **Old fork retired** (2026-09-23): the live acceptance run passed, opencode
   uses the published package, and the local clone was removed.
