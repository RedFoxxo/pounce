# pounce 🦊

An MCP server for [Targetprocess](https://www.ibm.com/products/targetprocess): lets AI
assistants read and manage your cards through the Targetprocess REST API.

> **Status: not implemented yet.** This repository currently contains the design
> and the API research only. Nothing below is usable until the implementation
> plan in [`CLAUDE.md`](CLAUDE.md) is carried out.

## Goals

- **Full coverage.** If Targetprocess exposes an operation, pounce can perform it.
  Generic tools are driven by your instance's own API metadata, so custom entity
  types and future Targetprocess versions are covered automatically.
- **Tools that fit how teams work.** Workflow tools accept names instead of ids
  ("move #36400 to Coded", "assign Leszek as Developer") and apply the team's
  rules: effort is booked per role, default assignees are cleared on creation,
  and side effects on parent cards are reported.
- **Honest results.** Every list is fully paged, every error carries
  Targetprocess's own message, and every write is read back to confirm it stuck.
- **Simple permissions.** Tools are grouped by tier, so an MCP client needs one
  rule per tier:

  ```yaml
  pounce_read_*: allow
  pounce_write_*: ask
  pounce_delete_*: ask
  pounce_admin_*: deny
  ```

## Documentation

- [`CLAUDE.md`](CLAUDE.md): architecture, domain rules, conventions, the confirmed
  Targetprocess API surface, and the implementation plan.
- [`docs/research/`](docs/research/): API surveys used to build the catalog.

## Branches

| Branch | Purpose |
|---|---|
| `main` | Development. All work happens here. |
| `stable` | Released, known-good state. Updated from `main` only when a version is ready. |
