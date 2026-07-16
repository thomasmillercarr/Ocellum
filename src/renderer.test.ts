import { describe, expect, it } from "vitest";
import type { Character } from "./character";
import { rollTransform } from "./behaviour";
import { renderFrame, type Ctx2d } from "./renderer";

interface DrawCall {
  img: unknown;
  dx: number;
  dy: number;
  alpha: number;
}

function recorder() {
  const draws: DrawCall[] = [];
  const ops: string[] = [];
  const ctx: Ctx2d = {
    globalAlpha: 1,
    clearRect: () => ops.push("clear"),
    save: () => ops.push("save"),
    restore: () => ops.push("restore"),
    translate: () => ops.push("translate"),
    rotate: () => ops.push("rotate"),
    scale: () => ops.push("scale"),
    drawImage(img, dx, dy) {
      draws.push({ img, dx, dy, alpha: this.globalAlpha });
      ops.push("draw");
    },
  };
  return { ctx, draws, ops };
}

const sentinel = () => ({}) as CanvasImageSource;

function fakeCharacter(layers: Character["layers"]): Character {
  return { name: "fake", width: 192, height: 192, layers };
}

describe("renderFrame", () => {
  it("composites every layer at (0,0) — zero offset, no per-layer maths", () => {
    const character = fakeCharacter({
      body: sentinel(),
      shadow: sentinel(),
      eyes_open: sentinel(),
      eyes_half: sentinel(),
      eyes_closed: sentinel(),
      brows_happy: sentinel(),
    });
    const { ctx, draws } = recorder();
    renderFrame(ctx, character, "half", rollTransform(600, 72), ["brows_happy"]);
    expect(draws.length).toBe(4); // shadow, body, eyes_half, brows_happy
    for (const d of draws) {
      expect(d.dx).toBe(0);
      expect(d.dy).toBe(0);
    }
  });

  it("draws shadow first, then body, then the eye layer for the blink state", () => {
    const shadow = sentinel();
    const body = sentinel();
    const eyesClosed = sentinel();
    const character = fakeCharacter({
      body,
      shadow,
      eyes_open: sentinel(),
      eyes_half: sentinel(),
      eyes_closed: eyesClosed,
    });
    const { ctx, draws } = recorder();
    renderFrame(ctx, character, "closed", rollTransform(0, 72));
    expect(draws.map((d) => d.img)).toEqual([shadow, body, eyesClosed]);
  });

  it("degrades gracefully: missing optional layers are skipped", () => {
    const character = fakeCharacter({
      body: sentinel(),
      eyes_open: sentinel(),
      eyes_half: sentinel(),
      eyes_closed: sentinel(),
    });
    const { ctx, draws } = recorder();
    renderFrame(ctx, character, "open", rollTransform(0, 72), ["brows_flat"]);
    expect(draws.length).toBe(2); // body + eyes only; no shadow, no brows
  });

  it("applies the roll shadow opacity to the shadow draw only", () => {
    const character = fakeCharacter({
      body: sentinel(),
      shadow: sentinel(),
      eyes_open: sentinel(),
      eyes_half: sentinel(),
      eyes_closed: sentinel(),
    });
    const { ctx, draws } = recorder();
    const roll = rollTransform(600, 72);
    renderFrame(ctx, character, "open", roll);
    expect(draws[0].alpha).toBeCloseTo(roll.shadow.opacity);
  });
});
