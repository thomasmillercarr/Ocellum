// Compositing renderer. Every layer draws at (0,0) — registration is the
// asset's job (§8.2), never the renderer's. No per-layer position maths.
import type { Character, LayerName } from "./character";
import type { EyeState, RollTransform } from "./behaviour";

/** The 2D-context subset we use; tests inject a recorder. */
export interface Ctx2d {
  clearRect(x: number, y: number, w: number, h: number): void;
  save(): void;
  restore(): void;
  translate(x: number, y: number): void;
  rotate(rad: number): void;
  scale(x: number, y: number): void;
  drawImage(img: CanvasImageSource, dx: number, dy: number): void;
  globalAlpha: number;
}

const EYE_LAYER: Record<EyeState, LayerName> = {
  open: "eyes_open",
  half: "eyes_half",
  closed: "eyes_closed",
};

export function renderFrame(
  ctx: Ctx2d,
  character: Character,
  eyes: EyeState,
  roll: RollTransform,
  extraLayers: LayerName[] = [],
) {
  const { width, height } = character;
  const cx = width / 2;
  const cy = height / 2;
  ctx.clearRect(0, 0, width, height);

  // Shadow: its own transform, stays on the ground.
  const shadow = character.layers.shadow;
  if (shadow) {
    ctx.save();
    ctx.globalAlpha = roll.shadow.opacity;
    ctx.translate(cx + roll.shadow.translateX, cy);
    ctx.scale(roll.shadow.scaleX, 1);
    ctx.translate(-cx, -cy);
    ctx.drawImage(shadow, 0, 0);
    ctx.restore();
  }

  // Body stack: translate, then rotate+squash about the below-centre pivot.
  const px = cx;
  const py = cy + roll.pivotYOffset;
  ctx.save();
  ctx.translate(roll.translateX, 0);
  ctx.translate(px, py);
  ctx.rotate((roll.rotationDeg * Math.PI) / 180);
  ctx.scale(roll.scaleX, roll.scaleY);
  ctx.translate(-px, -py);

  const stack: LayerName[] = ["body", EYE_LAYER[eyes], ...extraLayers];
  for (const name of stack) {
    const layer = character.layers[name];
    if (layer) ctx.drawImage(layer, 0, 0);
  }
  ctx.restore();
}
