# Ocellum — Architecture

> Durable state for the builder. Rebuild your understanding from this file + PROGRESS.md.

## Status

Pre-M0. Toolchain installing (rustup + VS Build Tools via winget).

## Stack decisions

| Decision | Choice | Why |
|---|---|---|
| Shell | Tauri v2 (version pinned at install — see below) | Brief mandate; lightweight |
| Frontend | Vanilla TypeScript + Vite, no framework | Lightweight constraint; UI is one overlay window + bubble |
| DB | SQLite via rusqlite (Rust side) | Local-first, no ORM needed for 7 tables |
| Keychain | Windows Credential Manager via `windows` crate / keyring crate | Brief: creds never on disk |

## Pinned versions

Pinning mechanism: committed `Cargo.lock` + `package-lock.json` (exact
resolution of the whole graph). Headline versions resolved 2026-07-16:

- tauri 2.11.5, tauri-build 2.6.3, tauri-plugin-global-shortcut 2.3.2
- tao 0.35.3, webview2-com 0.38.2
- @tauri-apps/api 2.11.1, @tauri-apps/cli 2.11.4
- vite 7.3.6, typescript 5.9.3 — deliberately NOT vite 8 / TS 7 (brand-new
  majors; an autonomous run doesn't need new-major surprises)
- Rust stable-x86_64-pc-windows-msvc via rustup (minimal profile),
  VS 2022 Build Tools (VCTools workload)

## M0 interfaces

- Frontend → Rust commands: `set_hit_regions(rects: Rect[])` (logical,
  window-relative), `bubble_state(open: bool)`, `report_input(value: string)`.
- Rust → frontend events: `toggle-bubble` (tray/hotkey), `open-bubble`,
  `close-bubble` (test control).
- `Rect { x, y, w, h }` logical CSS px; Rust converts to physical per tick
  (outer_position + scale_factor), so moves/DPI changes need no re-report.
- Focus return: poll loop records `GetForegroundWindow()` while click-through
  (skipping our own hwnd); `bubble_state(false)` restores it via
  `SetForegroundWindow`. Raw user32 FFI, no `windows` crate dependency.
- Test control channel: `OCELLUM_TEST=1` → line protocol on 127.0.0.1:47613
  (hwnd / rects / ignore-state / bubble-open / input-value / open-bubble /
  close-bubble). Gate scripts drive real OS input against it.
- Global hotkey: Ctrl+Shift+O → toggle bubble.

## M0 core design: click-through with hit region

Tauri v2 has **no per-region hit-testing**. `set_ignore_cursor_events(true)`
maps to `WS_EX_TRANSPARENT` on Windows and applies to the whole window.

Pattern (standard for Tauri overlays):
- One oversized transparent always-on-top window.
- Frontend reports the current hit region rect(s) (pet circle; plus bubble
  rect when open) to Rust via a command.
- Rust polls `GetCursorPos` (~60Hz) and toggles ignore_cursor_events:
  cursor inside a hit rect → interactive; outside → click-through.
- Bubble open → hit region grows; close → shrinks. Focus: `set_focus()`
  on bubble open, and on close return focus by re-enabling click-through
  (WS_EX_TRANSPARENT windows never hold focus).

### Gate verification approach (M0)

- `WindowFromPoint(x, y)` at a coordinate over Ocellum:
  - click-through active → returns the window *below* (OS hit-testing skips
    WS_EX_TRANSPARENT). Asserts "window below receives the click".
  - over the hit region → returns Ocellum's HWND.
- Keystroke test: test-mode control channel (env `OCELLUM_TEST=1` starts a
  localhost-only control listener, not compiled into release behaviour paths
  users hit) lets the test script open the bubble, then SendInput types, then
  reads the input value back.

## Interfaces (owned, never delegated)

### M1 — character & behaviour

- `src/character.ts` — asset contract. `Character { name, width, height,
  layers: Partial<Record<LayerName, CanvasImageSource>> }`. Loaders:
  `characterFromBytes(manifestJson, layerBytes)` (validates via PNG IHDR
  before decode) and `placeholderCharacter()` (inline SVG data URLs, zero
  files). `validateLayerDimensions` refuses mis-registered characters.
- `src/behaviour.ts` — pure engine. `BlinkMachine(rng).at(tMs): EyeState`
  (Poisson gaps mean 4s clamped ≥800ms; double-blink = one event, 15%);
  `rollTransform(tMs, radius)` returns rotation/translate/squash/pivot +
  shadow params. Roll and blink share no state — independence is a tested
  invariant, don't "refactor" them onto one timer.
- `src/renderer.ts` — `renderFrame(ctx, character, eyes, roll, extraLayers)`.
  Draws every layer at (0,0); transforms are whole-canvas. `Ctx2d` interface
  exists so tests inject a recorder.
- Rust `read_character_dir(path)` — IO only (manifest string + base64 layer
  map); validation stays in the TS loader.
- Radius convention: 0.375·canvas width (placeholder's 72px on 192px).

### M2 — model layer

- `providers.rs` — `trait Provider { name, destination, model, stream(req, on_delta) -> ChatOutcome }`.
  Impls: Anthropic (SSE /v1/messages), OpenAI (SSE /v1/chat/completions,
  stream_options.include_usage), Ollama (JSON lines /api/chat), ClaudeCode
  (shell-out, stream-json). All base_urls injectable → gate tests run the full
  path against mock servers. Blocking reqwest on a worker thread — no async
  plumbing needed at this call volume.
- Claude Code lockdown: `--tools ""` disables the built-in set AND
  `--disallowedTools <list>` (belt and braces), `--max-turns 1`,
  `--no-session-persistence`. `build_claude_args` is pure and gate-asserted.
- `chat.rs::run_chat` — THE chokepoint. Order: spend-cap check → egress row
  (at send time; failed calls still egressed) → provider.stream → model_call
  cost row. Every model call goes through it; M3+ features must call it, not
  providers directly.
- `db.rs` — brief §7 schema verbatim (interaction.kind CHECK-constrained to
  the closed enum) + internal `setting` (never credentials) + `model_call`
  (cost ledger; interaction is lead-bound, budget must also meter lead-less
  calls). DB path: app_data_dir/ocellum.db, OCELLUM_DB_PATH override in tests.
- Keychain: `keyring` crate v4 → Windows Credential Manager, service
  "ocellum" ("ocellum-test" under OCELLUM_TEST). Keys never in DB/settings.
- Budget: `BudgetSummary` tagged enum — ApiKey{month_pence, cap...},
  ClaudeCode{calls, tokens in/out, note} (no remaining field, type-level),
  Free{calls}. Cap default ON at 500 pence.
- Prices: settings-stored JSON (editable), seeded 2026-07 (Anthropic from
  docs, OpenAI from public trackers). GBP via fixed configurable fx (0.79
  default) — no FX network calls (would be egress).
- `prompt.rs::build_system_prompt(tools)` — runtime assembly, no hardcoded
  tool list (§3); MCP (M5) feeds it.
- Default provider anthropic / claude-opus-4-8; user-changeable in Settings.

### M3 — the loop

- `leads.rs` — capture (heuristic parse: hand-rolled email scan, company
  from suffix words or domain stem, free-mail domains excluded), enrich
  (run_chat + web_search=true, refused on Ollama), draft (run_chat, writes
  interaction row + Windows clipboard via clipboard-manager plugin), local
  reminders (2s scanner thread), dismissals.
- **`surface(app, trigger_type, evidence, payload)` is the only door for
  unsolicited UI.** It enforces hard_silence and the three-dismissals rule
  and logs to SurfaceLog. New triggers (decay at M7, clipboard_lead) MUST go
  through it. Reminders are solicited: they bypass the dismissal counter but
  honour hard silence (deferred, not dropped — they stay pending).
- Clipboard monitoring: opt-in (default 0), 2s poll, surfaces
  "clipboard_lead" with the found email as evidence; frontend shows a red
  dot on the pet while active.
- Tray: Hard silence CheckMenuItem ↔ `hard_silence` setting.
- Events to frontend: enrich-done, draft-done, loop-error, surface,
  monitor-state.

### M4 — mood

- `src-tauri/src/mood.rs` — `derive_mood(conn) -> Mood` is a pure read over
  `lead`/`interaction`, computed on every request, never stored (§8.4):
  bright = draft in the last day; flat = ≥14 days since anything happened;
  restless = a lead with exactly one draft and no later touch, ≥5 days old
  (§7's decay framing); neutral otherwise — including an empty DB (a fresh
  install has nothing to be flat about). `record_mood_delta` appends to the
  `mood_event` journal (never read back into `derive_mood`); the only hook is
  draft creation in `leads.rs`. Frontend reads via `get_mood` command; test
  control channel exposes `mood`.
- `src/mood.ts` — mood → brow layer (`MOOD_BROWS`) passed as `extraLayers`
  to the existing renderer (absent layers degrade per §8.2, so no-brows
  characters need no special casing), and mood → roll speed via
  `moodRollRate`: main.ts advances a *virtual* roll clock by dt·rate so a
  mood change bends speed without a phase snap. Blink stays on wall time —
  roll/blink clock independence holds.

## Trade log

- (record capable-vs-light trades here)
