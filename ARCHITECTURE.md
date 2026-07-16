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

- TBD as milestones land.

## Trade log

- (record capable-vs-light trades here)
