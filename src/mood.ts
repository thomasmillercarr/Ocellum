// Mood → expression (§8.4). Mood is computed Rust-side from lead/interaction;
// this maps it onto the asset contract's brow layers and onto the one
// behaviour knob it may touch: the roll period. Blink stays untouched —
// roll/blink independence is a tested invariant.
import type { LayerName } from "./character";
import { ROLL_PERIOD_MS } from "./behaviour";

export type Mood = "bright" | "neutral" | "restless" | "flat";

export const MOOD_BROWS: Record<Mood, LayerName> = {
  bright: "brows_happy",
  neutral: "brows_neutral",
  restless: "brows_restless",
  flat: "brows_flat",
};

// ponytail: fixed scales, tuned by eye later; a character manifest field if
// characters ever want their own temperament.
const PERIOD_SCALE: Record<Mood, number> = {
  bright: 0.85,
  neutral: 1,
  restless: 0.7,
  flat: 1.4,
};

/** Effective roll period under this mood (restless rolls fast, flat slow). */
export function moodRollPeriod(mood: Mood): number {
  return ROLL_PERIOD_MS * PERIOD_SCALE[mood];
}

/**
 * Rate multiplier for the roll clock. The render loop advances a virtual
 * time by dt·rate so a mood change bends the roll speed without a phase
 * jump (changing the period against absolute time would make the ball snap).
 */
export function moodRollRate(mood: Mood): number {
  return ROLL_PERIOD_MS / moodRollPeriod(mood);
}
