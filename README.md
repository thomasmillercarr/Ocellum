# Ocellum

A desktop companion for individual salespeople. It lives in the Windows system
tray, appears on the desktop as a small animated character, and performs small,
high-frequency sales tasks using **your own LLM provider**.

**The core loop:**

```
capture (clipboard / paste / hotkey)
  → enrich (research the lead)
    → draft (personalised email)
      → remind (follow-up, scheduled locally)
```

## Principles

- **Local-first.** No backend, no accounts, no cloud sync. All state is a
  SQLite file on your machine.
- **No telemetry. None.** Not opt-in — none.
- **Egress log.** Every byte that leaves your machine is listed in a plain,
  readable panel: what, when, to where, why. Every row, no exceptions.
- **Bring your own model.** Calls go directly from your machine to the
  provider you chose, with your credentials, metered by a budget meter with a
  hard spend cap (on by default).
- **Outbound-only.** Ocellum never reads your email and never sends it —
  drafts go to your clipboard or a `mailto:` link. It is not a CRM.

## Providers

> **Important:** a Claude Pro or ChatGPT Plus *subscription* does **not**
> include API access — those are separate products with separate billing.
> Ocellum needs one of:

1. **API key** (default) — an Anthropic or OpenAI key from their developer
   console. Cheap for this workload; costs are shown to the penny.
2. **Claude Code** (detected, opt-in) — if you have the `claude` CLI installed
   and authenticated, Ocellum can shell out to it (locked down, single-turn,
   no tools). Note: this shares your Claude Code rate limit.
3. **Ollama** (optional) — local models. No web search, so enrichment is
   unavailable on this path.

Keys are stored in the Windows Credential Manager, never on disk.

## Status

Pre-release, built milestone by milestone with a hard gate per milestone —
see [PROGRESS.md](PROGRESS.md) and [ROADMAP.md](ROADMAP.md). M0–M4 (shell,
character, provider layer, the core loop, mood) have passed their gates. It
runs today with an animated character (the Sales Chameleon) and any of the
providers below; M5–M7 (MCP host, voice, the four features) are next.

## Unsigned builds

v1 ships unsigned: Windows SmartScreen will warn on first run. Click
**More info → Run anyway**. Code signing is planned if adoption justifies the
certificate cost.

## Building

Requires Rust (stable-msvc), VS 2022 Build Tools, Node 20+, and WebView2
(preinstalled on Windows 10/11).

```
npm install
npx tauri build
```

Gate scripts live in `scripts/` (`gate-m0.ps1` … per milestone) and must exit 0.

## Licence

AGPL-3.0 — see [LICENSE](LICENSE). External contributions will require a CLA
(coming with the contribution docs at v1).
