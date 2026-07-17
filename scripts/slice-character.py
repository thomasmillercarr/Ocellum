#!/usr/bin/env python3
"""Slice a 2x2 character pose sheet into registered Ocellum layers.

This is how the bundled Sales Chameleon was made, and how you can drop in your
own character. See the asset contract in the README and OCELLUM-BRIEF.md §8.2.

Expected sheet (one square PNG, four equal cells) on a solid magenta
(#FF00FF) background — magenta must not appear on the character itself:

    +-------------------+-------------------+
    | eyes OPEN         | eyes HALF-closed  |
    | (full character)  | (full character)  |
    +-------------------+-------------------+
    | eyes CLOSED       | (unused — a soft  |
    | (full character)  |  shadow is drawn) |
    +-------------------+-------------------+

Model: the whole character lives in each eye layer and `body` is left empty, so
blinking just swaps the eye layer (§8.2's composite stack draws body + one eye
layer). The three poses need only differ in the eyes; any drift in position is
corrected here by re-anchoring each to a common centre-x and feet (bottom) line.

The bottom-right cell is ignored — a clean ground shadow is drawn instead, which
looks better than an AI-generated one and is fully under our control.

Output: body.png (empty), eyes_open/half/closed.png, shadow.png, character.json,
all on an identical square canvas so they register with zero offset.

Usage:
    python scripts/slice-character.py [sheet.png] [out_dir] [--name NAME] [--preview]
Defaults: assets/Chameleon.png -> public/chameleon, name "Sales Chameleon".
"""
import argparse
import json
import os
from PIL import Image, ImageDraw, ImageFilter
import numpy as np

OUT = 720          # square layer canvas (2x the 96px..192px display size)
CX = OUT // 2      # common horizontal centre all poses are anchored to
FEET_Y = 650       # common ground line the feet sit on
SHADOW_Y = 655     # shadow centre, just under the feet


def key_magenta(im):
    """Return (rgba_with_alpha, foreground_mask). Magenta background is high
    R & B with very low green; character pinks/blues keep green >= 70."""
    a = np.array(im.convert("RGBA"))
    R, G, B = a[:, :, 0].astype(int), a[:, :, 1].astype(int), a[:, :, 2].astype(int)
    fg = ~((G < 70) & (R > 150) & (B > 150))
    a[:, :, 3] = np.where(fg, 255, 0).astype(np.uint8)
    return Image.fromarray(a, "RGBA"), fg


def make_shadow():
    """Soft black ground ellipse. Opaque here; the renderer scales its alpha
    0.25 -> 0.15 across the roll (§8.3)."""
    s = Image.new("RGBA", (OUT, OUT), (0, 0, 0, 0))
    ImageDraw.Draw(s).ellipse(
        [CX - 155, SHADOW_Y - 30, CX + 155, SHADOW_Y + 30], fill=(0, 0, 0, 255)
    )
    return s.filter(ImageFilter.GaussianBlur(8))


def anchored(keyed, mask, cell_xy, cell_wh):
    """Crop one cell's keyed character and paste it onto an OUT canvas so its
    (bbox centre-x, bottom-y) lands at the common (CX, FEET_Y)."""
    x0, y0 = cell_xy
    cw, ch = cell_wh
    ys, xs = np.where(mask[y0:y0 + ch, x0:x0 + cw])
    if len(xs) == 0:
        raise SystemExit(f"empty cell at {cell_xy} — is the sheet a 2x2 grid on magenta?")
    cx_local = (xs.min() + xs.max()) / 2
    cell = keyed.crop((x0, y0, x0 + cw, y0 + ch))
    canvas = Image.new("RGBA", (OUT, OUT), (0, 0, 0, 0))
    canvas.alpha_composite(cell, (int(round(CX - cx_local)), int(round(FEET_Y - ys.max()))))
    return canvas


def main():
    p = argparse.ArgumentParser(description="Slice a 2x2 pose sheet into Ocellum character layers.")
    p.add_argument("sheet", nargs="?", default="assets/Chameleon.png")
    p.add_argument("out_dir", nargs="?", default="public/chameleon")
    p.add_argument("--name", default="Sales Chameleon", help="character.json display name")
    p.add_argument("--preview", action="store_true", help="also write a registration-check PNG")
    args = p.parse_args()

    keyed, mask = key_magenta(Image.open(args.sheet))
    H, W = mask.shape
    cw, ch = W // 2, H // 2
    layers = {
        "eyes_open": anchored(keyed, mask, (0, 0), (cw, ch)),
        "eyes_half": anchored(keyed, mask, (cw, 0), (cw, ch)),
        "eyes_closed": anchored(keyed, mask, (0, ch), (cw, ch)),
        "shadow": make_shadow(),
        "body": Image.new("RGBA", (OUT, OUT), (0, 0, 0, 0)),  # empty; §8.2 stack
    }
    os.makedirs(args.out_dir, exist_ok=True)
    for name, img in layers.items():
        img.save(os.path.join(args.out_dir, f"{name}.png"))
    with open(os.path.join(args.out_dir, "character.json"), "w") as f:
        json.dump({"name": args.name}, f, indent=2)

    if args.preview:
        # Tint the three eye poses; a crisp single silhouette = well registered.
        chk = Image.new("RGBA", (OUT, OUT), (20, 20, 20, 255))
        for nm, col in [("eyes_open", (255, 0, 0, 90)), ("eyes_half", (0, 255, 0, 90)),
                        ("eyes_closed", (0, 0, 255, 90))]:
            a = np.array(layers[nm])[:, :, 3]
            tint = np.zeros((OUT, OUT, 4), np.uint8)
            tint[a > 40] = col
            chk.alpha_composite(Image.fromarray(tint, "RGBA"))
        prev = os.path.join(args.out_dir, "_registration_preview.png")
        chk.convert("RGB").save(prev)
        print("wrote", prev)

    print(f"wrote {len(layers)} layers + character.json to {args.out_dir}")


if __name__ == "__main__":
    main()
