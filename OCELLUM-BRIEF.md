# Ocellum — Build Brief

**Handoff document for autonomous build. Read fully before writing any code.**

Version 1.0 · Windows-first · AGPL-3.0 + CLA

---

## 0. How to use this document

You are building this in one pass with minimal human intervention. That is the goal, and these rules exist to make it survivable.

**Build in milestones. Do not skip ahead.** Each milestone in §9 has a gate: a command that must exit 0. Run it. If it fails, fix it. Do not proceed to the next milestone with a failing gate. Do not "come back to it later."

**Fail loudly.** If a milestone cannot pass its gate after honest effort, STOP. Write `BLOCKED.md` describing what failed, what you tried, and what you need. Do not work around it. Do not stub it out and continue. A build that stops at M2 with a clear blocker is worth more than a build that reaches M6 with four silent workarounds inside it.

**Questions go to a file, not to a prompt.** If something here is ambiguous, write the question to `DECISIONS-NEEDED.md`, state the assumption you're proceeding under, and continue. Do not halt for clarification unless you are blocked.

**Write durable state.** Maintain `ARCHITECTURE.md` (what exists, why, key interfaces) and `PROGRESS.md` (milestone status, gate results, decisions made). Update both at every milestone boundary. Assume your context will be compacted and you will need to reconstruct your own understanding from these files. They are for you, not for the human.

**Read the docs before writing shell code.** Tauri v1 → v2 was a substantial breaking change. Your training data likely contains v1 patterns that will fail in ways that look like your bug rather than a version mismatch. Before writing any Tauri code, fetch the current v2 documentation. Pin every dependency to the version that resolves at install time and record the versions in `ARCHITECTURE.md`. Do not use version ranges.

**Delegation policy.** Delegate to smaller/cheaper models freely, under one rule:

> **Delegate anything with a spec and a test. Never delegate anything that defines an interface.**

You retain: architecture, the MCP host, the provider abstraction, the behaviour engine, the character contract, anything touching the keychain or the egress log, and all gate verification.

Delegate: CRUD, settings UI, migrations, sprite/atlas tooling, test authoring, docs, refactors with a passing test suite.

Every delegation gets a written brief, an explicit file boundary it may not cross, and a test it must pass. Review the diff before commit. Never merge unreviewed.

---

## 1. What Ocellum is

A desktop companion for individual salespeople. It lives in the Windows system tray, appears on the desktop as a small animated character, and performs small, high-frequency sales tasks using the user's own LLM provider.

Local-first. No backend, no accounts, no cloud sync, no telemetry. All state is a SQLite file on the user's machine. All model calls go directly from that machine to the provider the user chose, with the user's credentials.

**The core loop — the only thing that must be excellent:**

```
capture (clipboard / paste / hotkey)
  → enrich (research the lead)
    → draft (personalised email)
      → remind (follow-up, scheduled locally)
```

Everything else is downstream of that loop working.

**Guiding constraint: lightweight.** Small installer, fast start, few dependencies, no heavy runtime. When a choice presents itself between capable-and-heavy versus adequate-and-light, take light. Record the trade in `ARCHITECTURE.md`.

## 2. Non-goals

Do not build these. Do not architect for them.

- **Sending email.** No SMTP, no OAuth mail scopes. Drafts go to the clipboard or a `mailto:` deeplink. The user sends from their own client.
- **Being a CRM.** No pipeline, no deal stages, no reporting, no lead status field. See §7.
- **LinkedIn scraping.** Never. LinkedIn content enters only as user-pasted text.
- **Any server component.** If a feature needs a server, it doesn't ship.
- **Teams / multi-user / sync.** One seat, one machine.
- **Semantic search / embeddings / vector store.** Not in v1. It is not needed and it is not light.

## 3. Architecture: Ocellum is an MCP host

The core ships: the window and tray shell, the character renderer and behaviour engine, the provider abstraction, the local store, the budget meter, the egress log, and the core-loop tools.

Nothing else. Every external capability — enrichment providers, calendar, CRM, news — is an MCP server the user mounts.

This keeps the core small enough for one maintainer, makes "write an MCP server" the obvious contribution path, and inherits the existing MCP ecosystem on day one.

**Consequence you must build for from the start:** the system prompt is assembled at runtime from whatever tools are currently mounted. Never hardcode a tool list. The prompt builder takes the mounted tool set as input.

## 4. Model layer

### 4.1 Provider paths

The premise "connect your Claude/ChatGPT subscription" is false and must not appear in the UI or README. A Claude Pro or ChatGPT Plus subscription does not grant API access; those are separate products with separate billing. Be accurate about this in onboarding — it is the single most likely source of user confusion.

Three paths, in priority order:

1. **API key (default).** User pastes an Anthropic or OpenAI key. Real friction — requires a console account and a card — but cheap for this workload and reliable.
2. **Claude Code (detected, opt-in).** If the `claude` binary is on PATH and authenticated, offer it. Shell out via `claude -p "<prompt>" --output-format stream-json`. Lock down `--disallowedTools` aggressively — it is an agent, not a completion endpoint, and left unconstrained it will attempt filesystem and shell operations. Claude Code runs natively on Windows 10/11 via PowerShell; WSL is not required. **Never bundle credentials, never circumvent auth.** Ocellum shells out to a CLI the user installed and pointed at it. Keep it opt-in and describe plainly in the README what it does.
3. **Ollama (optional).** Available through the provider abstraction at no cost to you. Not recommended, not default, nothing depends on it.

### 4.2 Task routing

| Task | Model | Rationale |
|---|---|---|
| Transcription | Local whisper.cpp, always | Excellent locally, free, no egress |
| Everything else | User's chosen provider | Classification is fractions of a penny; a local model tier is not worth a multi-GB dependency |

Deliberate reversal of an earlier plan to route classification to a local small model. Under the lightweight constraint, the token saving does not justify the dependency.

### 4.3 Zero-config value

Ocellum must do something useful before any key is entered. Voice notes, reminders, and decay tracking require no provider. The key prompt appears the first time the user requests a **draft** — in context, after value is demonstrated. Never a cold first-run configuration wall.

**Hard requirement: first run → first useful output in under three minutes.** Onboarding friction is this product's main competitor.

### 4.4 Web search

Use the provider's native web search (Anthropic and OpenAI both offer one) for enrichment. No separate Exa/Tavily key. Note in the UI that enrichment is unavailable on the Ollama path — Ollama has no web search.

## 5. Budget meter

The largest objection to bring-your-own-key is fear of an unbounded bill. Two modes:

**API-key mode.** Real token counts, real cost, shown in GBP, per feature, this month. A configurable hard spend cap, **on by default**, that stops calls when hit.

**Claude Code mode.** You cannot read the user's remaining rate-limit window — it is not exposed. Do not invent one. Report only Ocellum's own consumption: calls made, tokens used, this session and this month. Label it explicitly as *Ocellum's usage*, not *your remaining limit*. Warn in onboarding that Ocellum shares the user's Claude Code rate limit and heavy use may consume the window they need for other work.

## 6. Trust

A tool that reads the clipboard and the microphone and posts to a third-party API is a surveillance-shaped object. The open-source audience is correctly suspicious. Both of these are requirements, not features.

**Egress log.** A plain, readable panel: what left this machine, when, to where, why. Every row, no exceptions. Cheap to build; it is the entire trust story.

**Telemetry: none.** Not opt-in. None. It is worth more in the README than the data is worth in a dashboard.

## 7. Data model

Ocellum is **outbound-only**. It does not read email, so it knows what the user drafted and nothing about what happened next. Do not invent an outcome field, a status column, or UI to set one. That is the CRM this product refuses to be.

Deal decay is therefore reframed, and this is the correct framing, not a compromise: **"you drafted this and never sent a second touch."** That is knowable from local data and is honest.

```sql
lead(id, name, company, email, domain, source, raw_capture, created_at)
enrichment(id, lead_id, provider, payload_json, fetched_at, ttl)
interaction(id, lead_id, kind, body, model_used, tokens_in, tokens_out, cost_pence, created_at)
reminder(id, lead_id, due_at, note, state, fired_at)
dismissal(id, trigger_type, context_hash, created_at)
mood_event(id, delta, reason, created_at)
egress_log(id, destination, bytes, purpose, created_at)
```

`interaction.kind` is a closed enum. Do not extend it without writing to `DECISIONS-NEEDED.md`:

```
draft_email | voice_note | enrichment | triage | roleplay
```

Decay query: leads with exactly one `draft_email` interaction and nothing since, older than N days.

`dismissal` and `egress_log` are load-bearing — see §8.3 and §6.

## 8. The character

### 8.1 Build it without the art

The character is an **asset contract**, not a dependency. The behaviour engine emits state; the renderer draws whatever layers satisfy the contract. Ship with a placeholder ball drawn in code (SVG, four layers, no external assets). Real artwork is a folder drop later.

This makes characters themeable — anything satisfying the manifest works — which is a free community surface. It also removes art from the critical path entirely.

**Deferring the art does not defer the mood.** Mood is a data structure (§8.4) and ships at M4 regardless of what is drawn.

### 8.2 Asset contract

A character is a directory containing `character.json` and PNG layers.

```
character.json
body.png            everything except eyes and mouth
shadow.png          ground shadow, separate
eyes_open.png
eyes_half.png
eyes_closed.png
mouth_closed.png    optional
mouth_open.png      optional
brows_neutral.png   optional
brows_happy.png     optional
brows_restless.png  optional
brows_flat.png      optional
```

**Minimum viable character: `body`, `eyes_open`, `eyes_half`, `eyes_closed`.** Everything else degrades gracefully — a character without brows renders without mood expression; the mood data still exists and still drives behaviour.

**Registration is mandatory and is the most common failure.** Every layer exports on an identical canvas at identical dimensions, registered to a common origin, so compositing is `drawImage(layer, 0, 0)` per layer with no offset table. Transparent PNG, 2× display size. Validate registration at load: if canvas dimensions differ across layers, refuse to load the character and log why.

### 8.3 Motion

Roll is a **transform, not frames**. Do not render rotation as sprites.

- **Roll:** rotation ±10°, translateX ±6px, period ~2.4s, sine easing. Pivot at `(centre_x, centre_y + 0.4 · radius)` — *below* centre. Pivot at true centre reads as spinning in the air; below centre reads as tipping and rolling.
- **Squash:** at each roll extreme, scaleY 0.97 / scaleX 1.03, eased with the rotation.
- **Shadow:** translateX at 0.5× the body's, scaleX 0.9→1.1 across the roll, opacity 0.25→0.15 as it leans away.
- **Blink:** Poisson timer, mean 4s, minimum gap 800ms. Sequence `open → half (40ms) → closed (60ms) → half (40ms) → open`. 15% chance of an immediate double-blink.
- **Roll and blink must never share a clock.** Independent timers are the entire difference between alive and mechanical. If they sync, it reads as a screensaver.

### 8.4 Mood is load-bearing

Mood is a function of real local data — untouched leads, drafts without a second touch, days since last outbound. Not decoration.

This is the only defensible reason to build a character rather than a command palette. If mood ends up cosmetic, the honest version of this product is a Raycast extension. Keep it wired to the data.

### 8.5 Interruption policy — read twice

Clippy is a byword for failure for a precise reason: **it interrupted without earned relevance.** This is an architectural constraint.

- **Silent by default.** Ocellum speaks only when (a) summoned, or (b) it has a specific, evidenced, dismissible reason.
- **Evidence is attached to every unsolicited surface.** *"You copied an address from a company you researched yesterday"* — never *"It looks like you're writing an email."*
- **Three dismissals of a trigger type disables that trigger permanently**, silently, no nag. This is what `dismissal` is for.
- **Hard silence toggle** in the tray menu, honoured absolutely.
- **Clipboard monitoring is opt-in**, off on first run, with a visible indicator when active.

### 8.6 Window and bubble

**One window.** The pet occupies a fixed corner of an oversized transparent window; the bubble occupies the rest and is hidden until the pet is clicked. This eliminates the entire class of multi-display anchoring bugs a second window creates.

- Transparent, always-on-top, click-through **except** the current hit region.
- The hit region grows when the bubble opens, shrinks when it closes.
- **Trap:** a click-through window cannot take keyboard focus, so text input in the bubble dies silently. Focus must be acquired on bubble open and surrendered on close. This is an M0 gate criterion, not an M3 discovery.
- Bubble flips to the opposite side of the pet when it would overflow the screen edge.

## 9. Milestones

Each gate must exit 0 before proceeding. Write the gate script as part of the milestone.

---

### M0 — The Windows spike ⚠ HIGHEST RISK, BUILD FIRST

Tauri v2 transparent + always-on-top + click-through-except-hit-region on Windows. Tray icon with menu. Global hotkey summon. Bubble open/close with focus acquisition.

This is the highest-uncertainty component in the stack. If it does not work, the shell choice is wrong and everything downstream is built on sand. **Do not design anything else until this passes.** Do not build five milestones of easy work and discover this at M5.

Windows specifics: click-through is `WS_EX_TRANSPARENT` under Tauri's `set_ignore_cursor_events`. Expect DPI scaling issues on multi-monitor.

**Gate:**
- App launches, tray icon present, menu opens, quit works
- A placeholder shape renders on the desktop with a transparent background
- Automated test: click at a coordinate outside the hit region → the window below receives it
- Automated test: click inside the hit region → Ocellum receives it
- Automated test: bubble opens, a text input receives keystrokes, bubble closes, focus returns
- Survives sleep/wake and a display resolution change without visual corruption
- Manual check recorded in `PROGRESS.md`: behaviour on a second monitor at a different DPI

---

### M1 — Signs of life

Character contract loader, placeholder ball character, behaviour engine, compositing renderer.

**Gate:**
- `character.json` + layers load; mismatched canvas dimensions are rejected with a clear error
- Unit test: blink state machine emits `open → half → closed → half → open` with specified durations
- Unit test: over 1000 simulated seconds, blink intervals fit a Poisson distribution with mean 4s ± 10%, no gap under 800ms
- Unit test: roll transform at t returns expected rotation/translation/squash values; pivot y-offset is `0.4 · radius`
- Unit test: **roll phase and blink phase are statistically uncorrelated over 1000s**
- Renderer composites N layers with zero offset and no per-layer position maths
- Placeholder character requires no external asset files

---

### M2 — A brain

Provider abstraction, keychain storage, settings panel, streaming conversation in the bubble, budget meter, egress log.

**Gate:**
- The same prompt runs on Anthropic, OpenAI, and Ollama through one interface
- Claude Code detected on PATH, auth verified, used as a provider, `--disallowedTools` asserted in test
- Keys are in the Windows Credential Manager; automated test asserts **no key appears in any file on disk**
- Cost is calculated and displayed to the penny in API-key mode
- Claude Code mode reports own-usage and does **not** display a remaining-limit figure
- Spend cap is on by default; test asserts calls stop when it is hit
- Every model call produces an `egress_log` row; test asserts count parity between calls and rows

---

### M3 — The loop

Clipboard capture, lead record, enrichment via provider web search, email drafting, local reminders, dismissal tracking.

**Gate:**
- End-to-end test: text in → lead created → enriched → draft produced → reminder scheduled → reminder fires
- Clipboard monitoring is off on first run; test asserts default state
- Test: three dismissals of one trigger type → that trigger no longer fires
- Test: hard silence toggle suppresses all unsolicited surfaces
- Drafting writes to clipboard; test asserts **no network call to any SMTP or mail API**
- Cold start to first draft, timed, recorded in `PROGRESS.md` — must be under 3 minutes including key entry

---

### M4 — Mood

Mood as a function of local data. Character expression driven by mood where the loaded character supports it.

**Gate:**
- Test: seed a DB with 14 days of no outbound → mood state is `flat`
- Test: log a fresh draft → mood delta is positive, `mood_event` row written
- Test: a character with no brow layers renders without error and mood state is still computed and still affects behaviour
- Mood is derived from `lead`/`interaction`, never stored as a settable field

---

### M5 — MCP host

Mount servers from config. Assemble the system prompt from mounted tools. One bundled example server.

**Gate:**
- A third-party MCP server is added to config and its tools become callable **with no code change**
- Test: system prompt content changes when a server is mounted/unmounted
- Test: no tool list is hardcoded anywhere — grep-based assertion
- A failing/unreachable MCP server degrades gracefully; it does not crash the app

---

### M6 — Voice

whisper.cpp, model downloaded on demand (**not bundled** — keeps the installer small), push-to-talk hotkey, note → draft.

**Gate:**
- First voice use triggers a model download with a progress indicator; installer size is unaffected
- 40s of speech → structured note + draft, **offline**, in under 10 seconds on the reference machine
- Test: transcription produces **zero** `egress_log` rows

---

### M7 — The four features

Reply triage, deal decay alerting, objection roleplay, draft critique.

Cheap by design: roleplay is pure prompt; decay is a SQL query plus M3's notification path; triage is a prompt plus the existing bubble UI.

**Gate:**
- Triage: an inbound reply → classified objection + **three responses with distinct postures** (concede / reframe / hold). Test asserts three distinct strategies, not three tones.
- Decay: test seeds a lead with one draft and no second touch → alert fires with the correct framing. Test asserts no outcome/status field is read or written.
- Roleplay: multi-turn session maintains prospect persona across at least 5 turns
- Critique: returns a score plus specific edits, **never a rewrite**

---

## 10. Distribution

**Licence: AGPL-3.0**, with a CLA required for external contributions. Ship `LICENSE`, `CLA.md`, and a `CONTRIBUTING.md` explaining both. This is unchangeable after the first external contribution without tracing every contributor — treat it as settled.

**Code signing:** unsigned Windows builds get SmartScreen-blocked. A cert is roughly £200/yr. For v1, **ship unsigned and document the warning and the bypass clearly in the README.** Revisit if adoption justifies it. Note this in `DECISIONS-NEEDED.md` for the human.

**Platform order:** Windows → macOS → Linux (community-supported, never blocking, no guarantees). Do not spend time on Linux transparency; it is compositor-dependent and the addressable audience rounds to zero.

**Reserve the npm name and the domain** before first release.

## 11. Audience — this changes defaults

The realistic v1 audience is **technical people who sell**, not salespeople generally. Salespeople do not install Ollama and mostly do not have API keys.

Implications, already reflected above but stated so they are not undone:

- API key is the default path. Ollama is not.
- Zero-config value exists so the product is useful before the key conversation.
- Onboarding friction is the main competitor, not other tools.

## 12. Open items for the human

Write anything you discover to `DECISIONS-NEEDED.md`. Currently open:

1. Code signing — ship unsigned for v1? (§10)
2. Default qualification rubric for note extraction — MEDDIC, BANT, SPICED, or none?
3. Character artwork — not required until after M4; placeholder ships until then.
