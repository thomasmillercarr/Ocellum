# Decisions needed (for the human)

1. **Code signing** — shipping unsigned for v1 per brief §10; SmartScreen warning
   will be documented in README. Confirm or fund a cert (~£200/yr).
2. **Qualification rubric** for note extraction — MEDDIC / BANT / SPICED / none?
   (Not needed until M6/M7; no assumption made yet.)
3. **Test control channel** — gates require automated keystroke/click tests.
   Assumption: a localhost-only control listener compiled in but active *only*
   when `OCELLUM_TEST=1` is set. It is not a server component in the product
   sense (nothing user-facing depends on it). Proceeding under this assumption.
