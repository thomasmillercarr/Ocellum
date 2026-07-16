# Ocellum — Progress

## Milestone status

| Milestone | Status | Gate result |
|---|---|---|
| M0 — Windows spike | **PASSED** 2026-07-16 | `scripts/gate-m0.ps1` exit 0 — all 13 automated checks pass |
| M1 — Signs of life | not started | — |
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

## M0 gate result (2026-07-16)

All automated checks pass against the release exe:
launch, control channel, hit regions reported, click-through ON outside
region (OS WindowFromPoint resolves to window below), interactive over pet
(WindowFromPoint resolves to Ocellum), bubble opens with focus, text input
receives real SendKeys keystrokes, bubble closes, focus returns to previous
window. Resolution-change check SKIP (see manual checks).
