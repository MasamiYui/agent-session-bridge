# Compatibility and verification

Validation date: 2026-10-10. Local host: macOS, Apple Silicon.

| Component | Version / format | Evidence and limits |
| --- | --- | --- |
| Codex CLI / app-server | 0.160.1 | Imported legacy rollout registered with `thread/resume(path)`, discovered with `thread/read`, resumed and sent imported history to a local Responses mock. No direct Codex SQLite writes. |
| Claude Code | 2.1.280 | Converted JSONL discovered using `--bare --resume <id>` in the canonical project directory; request to local Messages mock included imported history and completed a new turn. |
| Codex input | Legacy and paginated JSONL | Regression fixtures cover duplicate event/response messages, mixed item-only messages, tools, reasoning, compaction, malformed records. |
| Claude input | Parent-linked JSONL | Branch recovery, split assistant blocks, empty chunks, tool pairing and raw sidechain archive tests. |
| Archive | `.asb` manifest v1 | Byte-exact primary transcript round trip; attachment and sidechain re-export; unsafe extraction and overwrite rejection. |
| Other CLI versions | Not verified | Native mode disabled. Browsing, export and handoff remain available. |
| Windows / Linux | CI configured | No claim of local runtime validation. See actual GitHub Actions results for build status. |

`npm run test:integration` starts a localhost-only mock server on an ephemeral port and uses fresh temporary Bridge/Codex/Claude directories. It asserts the recorded model **request history**, not merely a fixed mock response. No user account credentials are needed. Evidence logs remain in the printed temporary directory and are not committed.

Core performance coverage parses a transcript larger than 50 MB and indexes 1,000 sessions. This is a functional stress check, not a latency or peak-memory guarantee: normalized messages are materialized on preview, so large files still use proportional memory. SQLite search and file parsing run outside the GUI thread; the reader renders messages incrementally.

GUI tests exercise search/preview/tool expansion, organization and restore, language/theme switching and batch handoff with native mode blocked for unverified versions. Synthetic UI tests do not replace native webview testing or CLI integration.

Recovery uses an operation intent before native target publication and a durable result before the SQLite completion update. Startup inspects only the operation's own target ID. It preserves ambiguous/interrupted files and records an actionable failure instead of deleting potentially continued history.

Known non-portable state: encrypted reasoning, runtime permissions, active tool execution, subagent topology, image context and exact compaction boundaries. Such content is either explicitly degraded in the conversion report or retained only in the complete source archive.
