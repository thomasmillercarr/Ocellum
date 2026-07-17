# Ocellum — Roadmap M4–M7

Gates are defined in [OCELLUM-BRIEF.md](OCELLUM-BRIEF.md) §9 and are binding.
Delegation rule (brief §0): *delegate anything with a spec and a test; never
delegate anything that defines an interface.* Every delegated task gets a
written brief, a file boundary it may not cross, and a test it must pass;
diffs are reviewed before commit.

## M4 — Mood

Mood as a pure function of local data (`lead`/`interaction`), never a stored
field. One new Rust module + frontend wiring.

| Task | Owner | Notes |
|---|---|---|
| `src-tauri/src/mood.rs`: `derive_mood(conn)` → flat / restless / bright / neutral | **owned** | flat = ≥14 days no outbound; restless = drafts without a second touch (§7 decay query shape); bright = fresh outbound |
| `mood_event` delta on draft creation (hook in `leads.rs`) | **owned** | table already exists in db.rs |
| Frontend: mood → brow layer via existing `renderFrame` extraLayers; behaviour modulated only through existing knobs | **owned** | blink/roll timers stay independent — tested invariant |
| `scripts/gate-m4.ps1` + tests | **owned** | incl. grep assertion that mood is derived-only (no column, no setter) |

## M5 — MCP host

The core interface work of the release. Mount servers from config; system
prompt assembled from mounted tools (`prompt.rs` already takes the tool set).

| Task | Owner | Notes |
|---|---|---|
| MCP client: stdio transport, JSON-RPC initialize / tools/list / tools/call (`src-tauri/src/mcp.rs`) | **owned** | rmcp crate vs hand-rolled ~200-line client: take lighter, record trade in ARCHITECTURE.md |
| Tool-use round-trip in `chat.rs::run_chat` (tool_use → MCP dispatch → tool_result → continue) | **owned** | largest single task in M5–M7; providers currently stream text only |
| Server config format, mount/unmount → prompt rebuild | **owned** | |
| Bundled example MCP server | delegated | spec: one tool, stdio; boundary: new file only |
| gate-m5.ps1: mount-without-code-change, prompt-changes-on-mount, grep no-hardcoded-tool-list, unreachable-server-degrades | delegated | gate verification itself reviewed by owner |

## M6 — Voice

whisper.cpp, model downloaded on demand (never bundled), push-to-talk,
note → draft. Transcription is local: **zero** egress rows, gate-asserted.

| Task | Owner | Notes |
|---|---|---|
| whisper integration (whisper-rs vs shell-out — take lighter) + audio capture | **owned** | egress guarantee is owned territory |
| Push-to-talk hotkey | **owned** | existing global-shortcut plugin |
| Model download-on-demand + progress UI | delegated | test: installer size unaffected |
| Note → draft prompt | delegated | rubric: **none for v1** — free-form who / company / pain / next step |
| gate-m6.ps1: 40s speech → note + draft, offline, <10s reference machine | delegated | |

## M7 — The four features

Cheap by design: each rides existing rails — `run_chat` for model calls,
`surface()` for unsolicited UI (enforces hard-silence + three-dismissals),
the bubble for interaction.

| Task | Owner | Notes |
|---|---|---|
| Triage: reply → objection + 3 responses with distinct postures (concede / reframe / hold) | delegated | test asserts distinct strategies, not tones |
| Decay: §7 SQL + `surface("decay", evidence…)` | delegated | test asserts no outcome/status field exists |
| Roleplay: multi-turn persona, ≥5 turns | delegated | conversation state across turns; session-state design reviewed by owner |
| Critique: score + specific edits, never a rewrite | delegated | |
| gate-m7.ps1 verification of all four | **owned** | |

## Post-M7 (v1 ship, not scheduled)

CLA.md + CONTRIBUTING.md, README polish, installer/release with documented
SmartScreen bypass, reserve npm name + domain (brief §10).
