import { describe, expect, it } from "vitest";
import type { Character } from "./character";
import { ROLL_PERIOD_MS, rollTransform } from "./behaviour";
import { renderFrame, type Ctx2d } from "./renderer";
import { MOOD_BROWS, moodRollPeriod, moodRollRate, type Mood } from "./mood";

const MOODS: Mood[] = ["bright", "neutral", "restless", "flat"];

describe("mood expression", () => {
  it("maps every mood to a distinct brow layer", () => {
    const layers = MOODS.map((m) => MOOD_BROWS[m]);
    expect(new Set(layers).size).toBe(MOODS.length);
    for (const l of layers) expect(l).toMatch(/^brows_/);
  });

  it("modulates behaviour: each mood has a period, neutral is the baseline", () => {
    expect(moodRollPeriod("neutral")).toBe(ROLL_PERIOD_MS);
    expect(moodRollPeriod("flat")).toBeGreaterThan(ROLL_PERIOD_MS); // sluggish
    expect(moodRollPeriod("restless")).toBeLessThan(ROLL_PERIOD_MS); // fidgety
    expect(moodRollPeriod("bright")).toBeLessThan(ROLL_PERIOD_MS);
    // Rate is the inverse: the virtual roll clock runs fast when fidgety.
    for (const m of MOODS) {
      expect(moodRollRate(m) * moodRollPeriod(m)).toBeCloseTo(ROLL_PERIOD_MS);
    }
  });

  it("renders a character with no brow layers without error — mood still applies", () => {
    // Placeholder-shaped character: required layers only, no brows (§8.2
    // minimum viable character).
    const sentinel = () => ({}) as CanvasImageSource;
    const character: Character = {
      name: "no-brows",
      width: 192,
      height: 192,
      layers: {
        body: sentinel(),
        eyes_open: sentinel(),
        eyes_half: sentinel(),
        eyes_closed: sentinel(),
      },
    };
    const draws: unknown[] = [];
    const ctx: Ctx2d = {
      globalAlpha: 1,
      clearRect() {},
      save() {},
      restore() {},
      translate() {},
      rotate() {},
      scale() {},
      drawImage(img) {
        draws.push(img);
      },
    };
    for (const m of MOODS) {
      draws.length = 0;
      expect(() =>
        renderFrame(ctx, character, "open", rollTransform(300 * moodRollRate(m), 72), [
          MOOD_BROWS[m],
        ]),
      ).not.toThrow();
      expect(draws.length).toBe(2); // body + eyes; the brow layer degrades away
    }
  });
});
