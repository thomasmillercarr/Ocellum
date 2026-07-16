// Behaviour engine (§8.3). Pure and deterministic given an RNG — the
// renderer feeds it wall-clock time; tests feed it simulated time.
//
// Roll and blink deliberately share no clock or RNG state: roll is a pure
// function of t, blink is a Poisson-timed state machine. Independence is
// asserted statistically in tests.

export type EyeState = "open" | "half" | "closed";

// Blink sequence: open → half(40ms) → closed(60ms) → half(40ms) → open.
const HALF_MS = 40;
const CLOSED_MS = 60;
export const BLINK_SEQUENCE_MS = HALF_MS + CLOSED_MS + HALF_MS; // 140
// ponytail: 90ms of open between the two blinks of a double-blink — "immediate"
// per spec but distinguishable as two blinks. Tune by eye later if needed.
const DOUBLE_GAP_MS = 90;

export const BLINK_MEAN_GAP_MS = 4000;
export const BLINK_MIN_GAP_MS = 800;
const DOUBLE_BLINK_CHANCE = 0.15;

/** Exponential inter-arrival (Poisson process), clamped to the minimum gap. */
function nextGap(rng: () => number): number {
  const u = Math.max(rng(), Number.MIN_VALUE); // avoid ln(0)
  return Math.max(BLINK_MIN_GAP_MS, -BLINK_MEAN_GAP_MS * Math.log(u));
}

interface BlinkEvent {
  /** Start of the blink sequence (ms). */
  start: number;
  /** True if this event is a double-blink (two sequences). */
  double: boolean;
}

export class BlinkMachine {
  private rng: () => number;
  private events: BlinkEvent[] = [];
  private nextStart: number;

  constructor(rng: () => number = Math.random) {
    this.rng = rng;
    this.nextStart = nextGap(this.rng);
  }

  private eventDuration(e: BlinkEvent): number {
    return e.double ? BLINK_SEQUENCE_MS + DOUBLE_GAP_MS + BLINK_SEQUENCE_MS : BLINK_SEQUENCE_MS;
  }

  /** Extend the schedule to cover time t. */
  private scheduleTo(t: number) {
    while (this.nextStart <= t) {
      const e: BlinkEvent = { start: this.nextStart, double: this.rng() < DOUBLE_BLINK_CHANCE };
      this.events.push(e);
      // Min gap applies between blink *events*; a double-blink is one event.
      this.nextStart = e.start + this.eventDuration(e) + nextGap(this.rng);
    }
  }

  /** Eye state within a single blink sequence at offset ms (0..139). */
  private static phase(offset: number): EyeState {
    if (offset < HALF_MS) return "half";
    if (offset < HALF_MS + CLOSED_MS) return "closed";
    if (offset < BLINK_SEQUENCE_MS) return "half";
    return "open";
  }

  /** Advance to monotonic time t (ms) and return the eye state. */
  at(t: number): EyeState {
    this.scheduleTo(t);
    const e = this.events[this.events.length - 1];
    if (!e || t < e.start) return "open";
    let offset = t - e.start;
    if (offset < BLINK_SEQUENCE_MS) return BlinkMachine.phase(offset);
    if (e.double) {
      offset -= BLINK_SEQUENCE_MS + DOUBLE_GAP_MS;
      if (offset >= 0 && offset < BLINK_SEQUENCE_MS) return BlinkMachine.phase(offset);
    }
    return "open";
  }

  /** Blink event start times so far (tests: distribution + decorrelation). */
  eventStarts(): number[] {
    return this.events.map((e) => e.start);
  }

  /** Gaps between blink events (end of one to start of the next). */
  eventGaps(): number[] {
    const gaps: number[] = [];
    for (let i = 1; i < this.events.length; i++) {
      const prev = this.events[i - 1];
      gaps.push(this.events[i].start - (prev.start + this.eventDuration(prev)));
    }
    return gaps;
  }
}

// ---------------------------------------------------------------------------
// Roll — a transform, not frames (§8.3).
// ---------------------------------------------------------------------------

export const ROLL_PERIOD_MS = 2400;
const ROLL_MAX_DEG = 10;
const ROLL_MAX_TX = 6;
const SQUASH = 0.03;

export interface RollTransform {
  rotationDeg: number;
  translateX: number;
  scaleX: number;
  scaleY: number;
  /** Pivot is below centre: (cx, cy + 0.4·radius). */
  pivotYOffset: number;
  shadow: {
    translateX: number;
    scaleX: number;
    opacity: number;
  };
}

export function rollTransform(t: number, radius: number): RollTransform {
  const s = Math.sin((2 * Math.PI * t) / ROLL_PERIOD_MS);
  const lean = Math.abs(s);
  return {
    rotationDeg: ROLL_MAX_DEG * s,
    translateX: ROLL_MAX_TX * s,
    scaleX: 1 + SQUASH * lean,
    scaleY: 1 - SQUASH * lean,
    pivotYOffset: 0.4 * radius,
    shadow: {
      translateX: 0.5 * ROLL_MAX_TX * s,
      scaleX: 1.0 + 0.1 * s, // 0.9 → 1.1 across the roll
      opacity: 0.25 - 0.1 * lean, // 0.25 centred → 0.15 leaning away
    },
  };
}

/** Roll phase angle at t, radians in [0, 2π). For decorrelation tests. */
export function rollPhase(t: number): number {
  const p = ((2 * Math.PI * t) / ROLL_PERIOD_MS) % (2 * Math.PI);
  return p < 0 ? p + 2 * Math.PI : p;
}
