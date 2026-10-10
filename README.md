# Agent Session Bridge

A local desktop workspace for browsing, organizing and moving **Codex ↔ Claude Code** development sessions. Built with Tauri 2, React, TypeScript and Rust. [简体中文](README.zh-CN.md)

## Features

- Three-pane workspace with project/source/date filters, title and full-text search, newest/oldest sorting and multi-selection.
- Markdown reader with code, readable reasoning and expandable tool calls/results. Long conversations load in pages.
- Aliases, tags, favorites, archive, remove and restore stored in Bridge's own SQLite database. Original sessions are preserved.
- Native conversion creates a new session ID. Codex rollouts are registered through the official app-server; Bridge never writes Codex's SQLite database.
- Editable context handoff generated locally without a model call. Includes user requests, recent history, tool results and a full `.asb` backup.
- Import JSONL and `.asb` with per-file preview. Export Markdown, original JSONL or portable `.asb` ZIP archives. Batch migration/export supports progress, cancellation of queued items and migration retries.
- Chinese/English, system/light/dark themes, configurable source directories, CLI paths and export directory. Browsing/export works without either CLI.
- Incremental background scans every 15 seconds, on-demand preview and a manual refresh button.

## Run and build

Install Node.js 24, Rust via rustup (the repository pins 1.97.0), and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): Xcode command line tools on macOS, Microsoft C++ Build Tools and WebView2 on Windows, or WebKitGTK 4.1 and the listed Linux development libraries.

```sh
npm ci
npm run desktop           # native development app
npm run bundle            # production installers and app
```

Packages appear under `src-tauri/target/release/bundle/`: macOS `.app`/`.dmg`, Windows MSI/NSIS, Linux deb/AppImage. Local macOS packages are unsigned and unnotarized; use Finder's Open action if macOS requests confirmation. CI builds all three platforms and uploads artifacts; it does not publish a release.

```sh
npm run dev:demo          # synthetic browser preview; no local session access
npm run build
npm test
npm run test:core
npx playwright install chromium
npm run test:gui
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --all-targets -- -D warnings
npm run test:integration  # requires the exact supported CLIs; local mock model only
```

`VITE_DEMO=1` explicitly enables synthetic UI data. Do not set it for production builds. `ASB_DATA_DIR` overrides Bridge's data directory for isolated testing.

## Move a conversation

1. Set source directories and CLI executables in **Settings**. Defaults are `~/.codex` and `~/.claude`, respecting `CODEX_HOME` / `CLAUDE_CONFIG_DIR`.
2. Select a conversation or import `.jsonl` / `.asb`. Import first adds an owned copy to Bridge; choose **Bridge** to migrate it.
3. Choose the target tool, an existing project directory, and **Native session** or **Context handoff**. Review the conversion report or edit the handoff document.
4. Create the result. Copy the displayed resume command or open it in Terminal. Batch operations use the chosen project for all selected sessions.

Native conversion is enabled for **Codex 0.160.1** and **Claude Code 2.1.280**. Both were tested on macOS with isolated configuration directories and a localhost mock model, including the next request carrying imported history. Other versions can browse/export and use handoff; they are not marked native-compatible. See [compatibility and validation](docs/compatibility.md).

## Data and safety

Bridge uses its OS application data directory: `~/Library/Application Support/io.github.masamiyui.agent-session-bridge` on macOS, `%APPDATA%/io.github.masamiyui.agent-session-bridge` on Windows, and `$XDG_DATA_HOME/io.github.masamiyui.agent-session-bridge` (normally `~/.local/share/...`) on Linux.

The directory contains `bridge.sqlite`, settings, owned imports and operation journals/backups. Do not delete it if you need organization metadata or handoff archives. Session archives can contain private prompts, code, file paths and tool output; review them before sharing.

`.asb` v1 stores a versioned `manifest.json`, normalized conversation, SHA-256 checked `raw/session.jsonl`, collected attachments and Claude subagent JSONL files. Missing attachments are reported. Imported raw JSONL does not read arbitrary attachment paths on your computer. ZIP extraction rejects traversal, symlinks, duplicate/case-colliding names, reserved Windows names and oversized entries (256 MB per entry, 1 GB expanded archive, 10,000 entries).

Writes refuse to overwrite existing files and publish through temporary files/directories. Completed migrations are reused unless **Create another copy** is selected. Startup recovery restores durable results or verifies an interrupted native target; unresolved cases retain their files and block silent duplicate retries. Historical HTML/scripts and commands are never executed by the reader.

## Scope and limits

- Tool mappings include `Bash` ↔ `exec_command`; unsupported or unpaired tools become labeled historical text. Raw records remain in the backup.
- Encrypted reasoning, runtime permissions/state, subagent topology, image context and compaction boundaries are not reconstructed as live agent state. Sidechain raw files remain archival.
- Handoff includes all user text blocks limited to 2,000 characters each and the latest 24 messages limited to 6,000 characters per block. The complete archive is linked in the saved document.
- Native Claude conversion validates identity/JSONL structure on each write; the supported CLI version was tested separately with real resume/continuation. Bridge itself does not send a model request to verify each import.
- No cloud upload, cross-device synchronization or continuous two-way message synchronization. Opening a target CLI and continuing it uses that tool's own configuration and billing.

## Source layout

`src/` contains the GUI and typed command facade; `src-tauri/src/` contains parsing, the SQLite store, archives, migration, app-server transport and desktop commands. `src-tauri/tests/`, `tests/` and `scripts/integration-smoke.py` cover the core, GUI and real CLI workflows. The Apache-2.0 [license](LICENSE) is retained.
