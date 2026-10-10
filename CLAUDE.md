# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Local-first Tauri 2 desktop app that browses, organizes, and copies coding-agent sessions between Codex, Claude Code, and Antigravity. Original session stores are read, never rewritten in place. Organization metadata, owned imports, and migration journals live in Bridge's own SQLite database. There is no cloud sync and no model call for handoff generation.

Pinned toolchain: Node.js 24, Rust 1.97.0 (`rust-toolchain.toml`). `npm ci` then `npm run desktop` runs the native app; `npm run bundle` produces installers under `src-tauri/target/release/bundle/`.

## Commands

```sh
npm run desktop            # tauri dev (starts Vite on 127.0.0.1:1420)
npm run dev:demo           # browser-only UI with synthetic data; no local session access
npm run build              # tsc -b && vite build
npm run check              # tsc -b only
npm test                   # vitest, src/ only
npm run test:core          # cargo test --no-default-features (lib + src-tauri/tests/core.rs)
npm run test:gui           # playwright; needs `npx playwright install chromium` once
npm run test:integration   # real Codex 0.160.1 + Claude Code 2.1.280 against a localhost mock model
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --all-targets -- -D warnings
```

Run one frontend test with `npx vitest run src/ProjectDirectory.test.ts`. Run one Rust test with `cargo test --manifest-path src-tauri/Cargo.toml --no-default-features -- <test_name>`. CI (`.github/workflows/ci.yml`) runs build, vitest, fmt, clippy, core, and GUI on Ubuntu, then bundles macOS/Windows/Linux and uploads the installers. A `v*` tag also publishes a GitHub Release with those installers. `test:integration` is not in CI.

`VITE_DEMO=1` switches `src/api.ts` to an in-memory fake and must not be set for production builds. Playwright sets it and serves `npm run dev` at `http://127.0.0.1:1420`. `ASB_DATA_DIR` overrides the app data directory (`desktop.rs` setup); use it to isolate tests from `~/Library/Application Support/io.github.masamiyui.agent-session-bridge`.

Core tests and clippy deliberately use `--no-default-features`. The `desktop` feature (on by default) pulls in Tauri; the library `bridge_core` and the `probe` example are built without it.

## Architecture

Two layers, one shared model. `src/` is the React GUI. `src-tauri/src/` is the Rust core, compiled as library `bridge_core` (usable without Tauri) plus a `desktop`-feature binary.

**GUI (`src/`).** `App.tsx` is the whole workspace: three panes, filters, reader, and the settings/export/bridge/import/sync modals. `ProjectDirectory.tsx` groups sessions by project path; `ProjectSync.tsx` is the per-project sync panel; `i18n.ts` is the Chinese/English string table. Every backend call goes through `api()` in `src/api.ts`, which is `invoke(command, args)` against the Tauri commands in `desktop.rs`. In demo mode the same function name returns synthetic data, so UI work can be tested without a session store. Types in `src/types.ts` mirror the Rust structs and must stay in camelCase lockstep — Rust structs use `#[serde(rename_all = "camelCase")]`.

**Command boundary (`desktop.rs`).** All Tauri commands are here. SQLite and parsing run on `spawn_blocking`, never on the GUI thread. `AppState` holds the `Store`, a map of cancellable jobs, and one `operation_lock` shared by migrations and project sync. A background thread calls `Store::scan` every 15 seconds and emits `sessions-changed`; batch jobs emit `job-progress`. Startup runs `migration::recover` before serving commands.

**Store (`store.rs`).** `bridge.sqlite` (WAL) has `sessions` (summary JSON, organization JSON, mtime/size cache, `present` flag), an FTS5 `search` table, `operations` (migration journal), a single-row `settings` table, and `project_mappings`. `scan` walks four roots and re-parses a file only when mtime or size changed:

- Codex: `<codexHome>/sessions` and `archived_sessions` (`*.jsonl`); titles come from a read-only open of Codex's own `state_*.sqlite` `threads` table, which is the only thing Bridge reads there
- Claude: `<claudeHome>/projects` (`*.jsonl`, skipping `subagents/`)
- Antigravity: `<antigravityHome>/conversations` (`*.db`)
- Bridge-owned imports: `<data dir>/imports`

Defaults are `~/.codex`, `~/.claude`, `~/.gemini/antigravity`, overridable in settings and respecting `CODEX_HOME` / `CLAUDE_CONFIG_DIR`.

**Parsing (`parser.rs`).** JSONL is detected as Codex when any record has `type: session_meta`, otherwise Claude. Antigravity is a SQLite conversation whose payloads are decoded with a small hand-rolled protobuf reader (`parse_antigravity`); the same encoding is written back by `publish_antigravity` in `migration.rs`. A session `key` is `provider:id:<sha256(path)[:12]>` so it survives appended messages and distinguishes the same id in different stores. `fingerprint` is the SHA-256 of the raw bytes and is what migration uses to reject a source that changed after preview.

**Migration (`migration.rs`).** Two modes. `handoff` writes a locally generated Markdown context file (no model call) plus a `.asb` archive; limits are 2,000 characters per user block and the latest 24 messages at 6,000 characters per block. `native` creates a new session id and new files: Claude JSONL under the project slug directory, a Codex rollout JSONL that is then registered through the Codex app-server (`rpc.rs`, JSON-RPC over stdio: `initialize`, `thread/resume`, `thread/read`), or a new Antigravity SQLite conversation. Native mode is refused unless `rpc::capability` sees an exact version string: `codex-cli 0.160.1` or `2.1.280 (Claude Code)`. Antigravity has no CLI version check and no resume command. Tool mapping is narrow (`Bash` ↔ `exec_command`, plus Claude's `Read`/`Edit`/`Write`/`Glob`/`Grep`); anything unpaired or unrecognized becomes labeled historical text.

Safety invariants enforced in `execute` and `recover`, and covered by `src-tauri/tests/core.rs`: an operation row is inserted `running` before any target file exists; `intent.json` is written before the target is published; writes go through `archive::atomic_write` and refuse to overwrite; a completed migration with the same identity (fingerprint + target + mode + cwd) is reused unless `duplicate` is set; recovery only inspects the operation's own target id, restores a durable `result.json`, and otherwise keeps the files and marks the operation `interrupted` so a retry cannot silently fork a copy.

**Project sync (`project_sync.rs`).** For one project directory, each eligible session (not archived, removed, or source-archived) is planned toward the other two providers. Copies Bridge itself created are classified `generated` and excluded from reverse sync; an unchanged completed copy is `synced` and skipped; a changed source fingerprint produces a new copy and never overwrites the source or a continued target. Execution reuses `migration::execute` through `start_project_sync`.

**Archives (`archive.rs`).** `.asb` v1 is a ZIP: `manifest.json`, normalized conversation, SHA-256 checked `raw/session.jsonl`, attachments, and Claude sidechain JSONL. Extraction rejects traversal, symlinks, duplicate and case-colliding names, reserved Windows names, and oversized entries (256 MB per entry, 1 GB expanded, 10,000 entries). Export formats are Markdown, original JSONL, and `.asb`.

## Things that are easy to get wrong

- Do not write to Codex's SQLite, Claude's session files, or an existing Antigravity conversation. New native output is always a new id and a new file, registered (Codex) or verified by re-parsing.
- A new native CLI version is not supported until its exact `--version` string is added in `rpc::capability` and `docs/compatibility.md` is updated. Until then the GUI must fall back to handoff; `native_available` comes from that check.
- `src/types.ts` and the `camelCase` serde structs in `model.rs` are a manual pair. Changing one without the other breaks the IPC boundary silently at runtime.
- The reader must not execute anything found in a transcript. Historical HTML, scripts, and commands are displayed only.
- Antigravity's running language server only indexes conversations that existed when it started, so a native copy is invisible until Antigravity restarts.
