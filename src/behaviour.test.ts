import { describe, expect, it } from "vitest";
import {
  BLINK_MIN_GAP_MS,
  BLINK_SEQUENCE_MS,
  BlinkMachine,
  rollPhase,
  rollTransform,
} from "./behaviour";

/** Deterministic RNG so statistical tests are stable. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

describe("blink state machine", () => {
  it("emits open → half(40ms) → closed(60ms) → half(40ms) → open", () => {
    // rng queue: first value = gap draw (0.5 → ~2772ms), second = double-blink
    // draw (0.9 → single), rest for future scheduling.
    const values = [0.5, 0.9, 0.5, 0.9];
    let i = 0;
    const machine = new BlinkMachine(() => values[i++ % values.length]);
    const start = -4000 * Math.log(0.5); // the first scheduled start
    expect(machine.at(start - 1)).toBe("open");
    expect(machine.at(start + 1)).toBe("half");
    expect(machine.at(start + 39)).toBe("half");
    expect(machine.at(start + 41)).toBe("closed");
    expect(machine.at(start + 99)).toBe("closed");
    expect(machine.at(start + 101)).toBe("half");
    expect(machine.at(start + 139)).toBe("half");
    expect(machine.at(start + 141)).toBe("open");
  });

  it("double-blink runs a second sequence immediately after the first", () => {
    // gap draw 0.5, double draw 0.05 (< 0.15 → double)
    const values = [0.5, 0.05, 0.5, 0.9];
    let i = 0;
    const machine = new BlinkMachine(() => values[i++ % values.length]);
    const start = -4000 * Math.log(0.5);
    expect(machine.at(start + 141)).toBe("open"); // brief open between the two
    expect(machine.at(start + BLINK_SEQUENCE_MS + 90 + 50)).toBe("closed");
    expect(machine.at(start + 2 * BLINK_SEQUENCE_MS + 90 + 10)).toBe("open");
  });

  it("over 1000s: Poisson-like gaps, mean 4s ± 10%, no gap under 800ms", () => {
    const machine = new BlinkMachine(mulberry32(42));
    machine.at(1_000_000);
    const gaps = machine.eventGaps();
    expect(gaps.length).toBeGreaterThan(150);
    const mean = gaps.reduce((a, b) => a + b, 0) / gaps.length;
    // Exponential clamped at 800ms has mean 800 + 4000·e^-0.2 ≈ 4074.
    expect(mean).toBeGreaterThan(3600);
    expect(mean).toBeLessThan(4400);
    expect(Math.min(...gaps)).toBeGreaterThanOrEqual(BLINK_MIN_GAP_MS);
    // Shape check: exponential has CV 1; the 800ms clamp pulls it down a bit.
    const sd = Math.sqrt(gaps.reduce((a, b) => a + (b - mean) ** 2, 0) / gaps.length);
    expect(sd / mean).toBeGreaterThan(0.7);
    expect(sd / mean).toBeLessThan(1.1);
  });

  it("roll phase and blink phase are statistically uncorrelated over 1000s", () => {
    const machine = new BlinkMachine(mulberry32(7));
    machine.at(1_000_000);
    const phases = machine.eventStarts().map((t) => rollPhase(t));
    expect(phases.length).toBeGreaterThan(150);
    // If blinks synced to the roll, onset phases would cluster: the mean
    // resultant vector length R would approach 1. Uniform → R near 0.
    const meanCos = phases.reduce((a, p) => a + Math.cos(p), 0) / phases.length;
    const meanSin = phases.reduce((a, p) => a + Math.sin(p), 0) / phases.length;
    const R = Math.hypot(meanCos, meanSin);
    expect(R).toBeLessThan(0.15);
  });
});

describe("roll transform", () => {
  const radius = 72;

  it("is neutral at t=0", () => {
    const r = rollTransform(0, radius);
    expect(r.rotationDeg).toBeCloseTo(0);
    expect(r.translateX).toBeCloseTo(0);
    expect(r.scaleX).toBeCloseTo(1);
    expect(r.scaleY).toBeCloseTo(1);
    expect(r.shadow.scaleX).toBeCloseTo(1.0);
    expect(r.shadow.opacity).toBeCloseTo(0.25);
  });

  it("peaks at quarter period: +10°, +6px, squash 1.03/0.97", () => {
    const r = rollTransform(600, radius); // 2400/4
    expect(r.rotationDeg).toBeCloseTo(10);
    expect(r.translateX).toBeCloseTo(6);
    expect(r.scaleX).toBeCloseTo(1.03);
    expect(r.scaleY).toBeCloseTo(0.97);
    expect(r.shadow.translateX).toBeCloseTo(3); // 0.5× body
    expect(r.shadow.scaleX).toBeCloseTo(1.1);
    expect(r.shadow.opacity).toBeCloseTo(0.15);
  });

  it("mirrors at three-quarter period: -10°, -6px, shadow 0.9", () => {
    const r = rollTransform(1800, radius);
    expect(r.rotationDeg).toBeCloseTo(-10);
    expect(r.translateX).toBeCloseTo(-6);
    expect(r.scaleX).toBeCloseTo(1.03);
    expect(r.scaleY).toBeCloseTo(0.97);
    expect(r.shadow.scaleX).toBeCloseTo(0.9);
    expect(r.shadow.opacity).toBeCloseTo(0.15);
  });

  it("pivot y-offset is 0.4 · radius", () => {
    expect(rollTransform(0, radius).pivotYOffset).toBeCloseTo(0.4 * radius);
    expect(rollTransform(123, 100).pivotYOffset).toBeCloseTo(40);
  });
});
