# Developing pounce

`CLAUDE.md` holds the architecture, domain rules, conventions and the confirmed
Targetprocess API surface, including the quirks found on a live instance. Read it
before changing behaviour.

## From source

Requires Node.js 22.12 or newer.

```sh
git clone https://github.com/RedFoxxo/pounce.git
cd pounce
npm ci
npm run build
```

The server entry point is `build/index.js` (MCP over stdio). To run a local build
instead of the published package, point your client at it, e.g. in opencode:

```json
"pounce": {
  "type": "local",
  "command": ["node", "/path/to/pounce/build/index.js"],
  "environment": { "TP_BASE_URL": "{env:TP_BASE_URL}", "TP_TOKEN": "{env:TP_TOKEN}" }
}
```

`.env.example` lists the variables. pounce does not read `.env` files itself.

## Commands

```sh
npm test            # unit + contract tests (stubbed fetch, no network)
npm run typecheck   # src and tests
npm run build       # compile to build/
npm run snapshot    # regenerate src/catalog/snapshot.json from TP_BASE_URL (read-only)
```

Run `npm run typecheck`, `npm test` and `npm run build` before every commit.

## Live tests

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

## Branches

| Branch | Purpose |
|---|---|
| `main` | Development. All work happens here. |
| `stable` | Released, known-good state. Updated from `main` only when a version is released. |

## Releasing

1. Bump the version in `package.json` and `src/version.ts` (a test keeps them
   equal), and move the changelog entry from "unreleased" to the release date.
2. Commit on `main`; `npm run typecheck`, `npm test` and `npm run build` must pass.
3. Fast-forward `stable` to `main` and tag the release: `git tag v<version>`.
4. Push `main`, `stable` and the tag.
5. `npm publish` (runs typecheck, tests and build first via `prepublishOnly`).
   Check the contents beforehand with `npm pack --dry-run`.
