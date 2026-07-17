# Decisions needed (for the human)

1. **Code signing** — DECIDED 2026-07-17: ship unsigned for v1; SmartScreen
   warning documented in README. Revisit if adoption justifies a cert (~£200/yr).
2. **Qualification rubric** — DECIDED 2026-07-17: none for v1. Voice-note
   extraction produces a free-form structured note (who / company / pain /
   next step), no framework jargon.
3. **Test control channel** — gates require automated keystroke/click tests.
   Assumption: a localhost-only control listener compiled in but active *only*
   when `OCELLUM_TEST=1` is set. It is not a server component in the product
   sense (nothing user-facing depends on it). Proceeding under this assumption.
