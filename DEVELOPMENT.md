# Developing pounce

`CLAUDE.md` holds the architecture, domain rules, conventions and the confirmed
Targetprocess API surface, including the quirks found on a live instance. Read it
before changing behaviour.

## Commands

pounce is a Rust crate (stable Rust, 1.85 or newer); nothing else is needed to
build or test it.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                        # unit tests and the replay, no network
cargo build --release             # target/release/pounce
```

Run the first three before every commit. CI (`.github/workflows/ci.yml`) runs
the same on every push to `main` and `stable` and on pull requests, plus
`cargo test` on Windows and macOS; it never runs the live tests.

To run a local build instead of an installed one, point your client at the
file, e.g. in opencode (under `mcp.servers`):

```json
"pounce": {
  "type": "local",
  "command": ["/path/to/pounce/target/release/pounce"],
  "environment": { "TP_BASE_URL": "{env:TP_BASE_URL}", "TP_TOKEN": "{env:TP_TOKEN}" }
}
```

`.env.example` lists the variables; pounce does not read `.env` files itself.
`pounce --help` lists them too.

## Replay and golden files

`cargo test` covers the configuration, the HTTP layer, the catalog, the
resolvers, the formats and the domain rules with unit tests, plus two golden
files:

- **`tests/golden/tools.json`** is the whole `tools/list`: names, parameters,
  descriptions and annotations. `lists_the_released_tools` (`src/tools/mod.rs`)
  compares it with what pounce lists. Tool names, parameters and descriptions
  are a contract with the Mamami plugin and with every model prompt, so a
  change there is deliberate and updates the golden file with it.
- **`tests/golden/transcripts/*.json`** (and `transcripts/tools/`) were recorded
  from the TypeScript build's tests (1.2.0, a fake Targetprocess, a fixed
  clock). Each holds every tool call with its arguments, every request it sent
  to Targetprocess with the answer it got, and the tool's result.
  `src/replay.rs` plays them back: pounce must send the same requests (in any
  order within one call) and give the same answers.

When behaviour changes on purpose, update the affected transcript entries by
hand: the requests the call now sends, with a plausible Targetprocess answer,
and the new result. The TypeScript recorder is gone, so there is nothing to
regenerate them with. Explain the change in the commit message; a replay
failure you did not intend is a regression, not a file to update.

## Live tests

Read-only checks against a real instance, the one in `TP_BASE_URL` with
`TP_TOKEN` (`src/live.rs`). They write nothing, are ignored by a plain
`cargo test`, and need one thread so they don't race each other's requests:

```sh
TP_BASE_URL=https://yourcompany.tpondemand.com TP_TOKEN=... \
  cargo test --bin pounce live:: -- --ignored --test-threads=1
```

There is no scripted write acceptance run: the TypeScript one (against a story
you named) is not ported, for lack of a test account. Check write changes by
hand on a test card.

## Snapshot

`src/catalog/snapshot.json` is the built-in catalog that stands in when the
instance's API metadata cannot be read. To refresh it from the instance in
`TP_BASE_URL` (read-only, about 270 requests), run from the repository root:

```sh
cargo run -- --snapshot
```

It overwrites `src/catalog/snapshot.json` in the current folder. Review the
diff: the catalog tests pin the resource counts.

## Measuring tools/list

Tool definitions are sent to the model on every request. After changing a tool
description or schema, compare the size of `tools/list` before and after, and
update the token figures under "Toolsets" in `README.md` (about 4 bytes per
token). With dummy settings, pounce answers without contacting Targetprocess:

```sh
cargo build
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"size","version":"0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' |
  TP_BASE_URL=https://example.invalid TP_TOKEN=x POUNCE_TOOLSETS=cards target/debug/pounce |
  tail -1 | wc -c
```

Leave `POUNCE_TOOLSETS` empty for all 58 tools. For 2.0.0 this gives 16,262
bytes for `cards` and 29,810 for all.

## Branches

| Branch | Purpose |
|---|---|
| `main` | Development. All work happens here. |
| `stable` | The default branch: the released state. Only the release workflow moves it. |

## Releasing

1. Bump `version` in `Cargo.toml` (the only place it lives) and run
   `cargo build` so `Cargo.lock` follows. Turn `## Unreleased` in the changelog
   into `## X.Y.Z — YYYY-MM-DD`.
2. Commit on `main`, tag `vX.Y.Z`, push both.
3. The `release` workflow checks that the tag matches `Cargo.toml` and the
   changelog, runs the CI checks on the tag, builds the five binaries and
   `pounce.mcpb` (`mcpb/build.sh`), starts the bundle on macOS and Linux
   (`mcpb/smoke.sh`), publishes a GitHub Release with the files, `SHA256SUMS`
   and the changelog section, moves `stable` to the release, then publishes
   `server.json` to the MCP Registry with the new version, bundle URL and
   checksum.
4. `cargo publish` puts the crate `pounce-mcp` on crates.io (needs the
   crates.io sign-in; not automated yet).

The MCP Registry signs in with the workflow's own GitHub identity, so no token
is stored; it only accepts names under `io.github.RedFoxxo/`. `server.json` in
the repository holds the listing's text and settings; the workflow fills in
the release.

`mcpb/manifest.json` is the bundle's manifest (the settings Claude Desktop asks
for); the build adds the version and the tool list from `tests/golden/`. On
macOS and Linux the bundle starts `/bin/sh server/launch.sh`, never the binary
directly: Claude Desktop on macOS extracts bundle files without the execute bit
(mcpb issue #294) and may keep the quarantine flag, and the launcher fixes both
and picks the Linux build for the CPU. `mcpb/smoke.sh` reproduces those
conditions.

The asset names (`pounce-linux-x64`, `pounce-linux-arm64`, `pounce-macos-x64`,
`pounce-macos-arm64`, `pounce-windows-x64.exe`) are a contract with
`install.sh` and `install.ps1`: never rename them. Linux builds are static
(musl), so one file runs on every distribution.

## Directories (Glama)

Glama lists pounce from the default branch, `stable`, which the release
moves forward, and scores the tool descriptions from `tools/list`. Descriptions
are graded per tool and the weakest weighs most: each says what the tool
answers, when to use it and which neighbouring tool instead, and every
parameter has a description. `glama.json` names the maintainers who can claim
the listing.

Glama does **not** build the repository's `Dockerfile` (that one is for
`docker build` users). It generates its own image from the form at
<https://glama.ai/mcp/servers/RedFoxxo/pounce/admin/dockerfile>, runs pounce
under `mcp-proxy` and lists its tools. The form's values live only on Glama;
this is their record, to paste back if the form is ever reset:

| Field | Value |
|---|---|
| Pinned commit SHA | the release's commit: `git rev-parse 'vX.Y.Z^{commit}'` |
| Placeholder parameters | `{"TP_BASE_URL": "https://example.invalid", "TP_TOKEN": "placeholder"}` |

Build steps:

```json
[
  "apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates curl && rm -rf /var/lib/apt/lists/*",
  "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal",
  ". \"$HOME/.cargo/env\" && cargo build --release --locked",
  "install -m 0755 target/release/pounce /usr/local/bin/pounce"
]
```

CMD arguments: `["mcp-proxy", "--", "pounce"]`. Glama's `mcp-proxy` passes the
container's environment to pounce, so the placeholder `TP_BASE_URL` and
`TP_TOKEN` reach it; without them pounce stops at startup with the missing
settings. Glama joins the build steps into one `RUN` after cloning the
repository into `/app`.

Environment variables JSON schema (pounce answers `tools/list` with only the
placeholders; it reaches Targetprocess on the first tool call):

```json
{
  "type": "object",
  "properties": {
    "TP_BASE_URL": { "type": "string", "description": "Your instance, e.g. https://yourcompany.tpondemand.com" },
    "TP_TOKEN": { "type": "string", "description": "Your Targetprocess access token" },
    "TP_DEFAULT_PROJECT_ID": { "type": "string", "description": "Project for new cards with no parent to inherit one from" },
    "TP_DEFAULT_TEAM_ID": { "type": "string", "description": "Team for new cards when none is given" },
    "POUNCE_TOOLSETS": { "type": "string", "description": "Tool groups to expose, comma-separated: cards, raw, admin (empty: all three)" }
  },
  "required": ["TP_BASE_URL", "TP_TOKEN"]
}
```

After each release: put the new commit in **Pinned commit SHA**, save, start a
build, and once it passes, make a release from the same page.

## Install scripts

`install.sh` (Linux, macOS) and `install.ps1` (Windows PowerShell 5.1 and
later) download a release file, check it against `SHA256SUMS` and install it.
Users run them from `main` (`curl … | sh`, `irm … | iex`), so a change there
reaches everyone at once; CI runs both against the latest release on Linux,
macOS and Windows, twice each (skipped until a release with the Rust binaries
exists).

The Mamami installer runs these scripts too. Their default folders
(`~/.local/bin`, `%LOCALAPPDATA%\Programs\pounce`) and settings
(`POUNCE_VERSION`, `POUNCE_INSTALL_DIR`, `POUNCE_NO_MODIFY_PATH`) are a
contract with it: keep them.
