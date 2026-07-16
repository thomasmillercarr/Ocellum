# Ocellum — Progress

## Milestone status

| Milestone | Status | Gate result |
|---|---|---|
| M0 — Windows spike | **PASSED** 2026-07-16 | `scripts/gate-m0.ps1` exit 0 — all 13 automated checks pass |
| M1 — Signs of life | **PASSED** 2026-07-16 | `scripts/gate-m1.ps1` exit 0 — 21 TS + 2 Rust tests, live render check |
| M2 — A brain | not started | — |
| M3 — The loop | not started | — |

## Log

- 2026-07-16: Build started. Read brief. No Rust/MSVC on machine; installing
  rustup (stable-msvc) + VS 2022 Build Tools in background. WebView2 present.
- 2026-07-16: Fetched Tauri v2 docs (tray, global-shortcut, window config,
  click-through). Confirmed: no per-region hit-testing in Tauri — cursor-poll
  + toggle `set_ignore_cursor_events` is the pattern. Recorded in
  ARCHITECTURE.md.

## Manual checks required by gates

- [ ] M0: second monitor at different DPI behaviour (record here)
- [ ] M0: sleep/wake survival — cannot be automated from inside the session
      (sleeping the machine suspends the test runner). Needs human check.
- [ ] M0: display resolution change — gate automates it via
      ChangeDisplaySettings, but this display enumerates no alternate modes,
      so it SKIPped. Rerun `scripts/gate-m0.ps1` on a machine with multiple
      modes, or check manually. ScaleFactorChanged handler repositions the
      window on DPI/display changes.
- [ ] M3: cold start → first draft timing (record here)

## M1 gate result (2026-07-16)

- Blink machine emits open→half(40)→closed(60)→half(40)→open; double-blink
  chains a second sequence after 90ms of open.
- 1000 simulated seconds (seeded RNG): gap mean 4.07s expected for the
  800ms-clamped exponential, within ±10% of 4s; min gap ≥ 800ms; CV in
  [0.7, 1.1] (shape check).
- Blink onsets vs roll phase: mean resultant length R < 0.15 (uniform).
- Roll transform: ±10° / ±6px / 1.03-0.97 squash / pivot 0.4·radius verified
  at t = 0, T/4, 3T/4. Shadow 0.5× translate, 0.9→1.1 scaleX, 0.25→0.15 alpha.
- Contract loader rejects mismatched canvas dims + missing required layers
  with clear errors (IHDR-level, before decode).
- Renderer: all layers drawn at (0,0); shadow→body→eyes order; optional
  layers degrade gracefully.
- Placeholder: all layers are data: SVG URLs — zero external assets.
- Live check: release exe launched, screen pixels at pet centre are the
  ball's blue (1465/1600) — the canvas really composites on the desktop.

## M0 gate result (2026-07-16)

All automated checks pass against the release exe:
launch, control channel, hit regions reported, click-through ON outside
region (OS WindowFromPoint resolves to window below), interactive over pet
(WindowFromPoint resolves to Ocellum), bubble opens with focus, text input
receives real SendKeys keystrokes, bubble closes, focus returns to previous
window. Resolution-change check SKIP (see manual checks).
