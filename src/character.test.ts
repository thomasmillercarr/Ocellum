import { describe, expect, it } from "vitest";
import {
  characterFromBytes,
  parseManifest,
  placeholderLayerUrls,
  pngDimensions,
  validateLayerDimensions,
} from "./character";

/** Minimal PNG header: signature + IHDR chunk with given dimensions. */
function pngHeader(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(24);
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
  const view = new DataView(bytes.buffer);
  view.setUint32(8, 13); // IHDR length
  bytes.set([0x49, 0x48, 0x44, 0x52], 12); // "IHDR"
  view.setUint32(16, width);
  view.setUint32(20, height);
  return bytes;
}

describe("pngDimensions", () => {
  it("reads width/height from IHDR", () => {
    expect(pngDimensions(pngHeader(192, 192))).toEqual({ width: 192, height: 192 });
    expect(pngDimensions(pngHeader(512, 256))).toEqual({ width: 512, height: 256 });
  });

  it("rejects non-PNG bytes", () => {
    expect(() => pngDimensions(new Uint8Array([1, 2, 3]))).toThrow("not a PNG");
  });
});

describe("validateLayerDimensions", () => {
  const d = (n: number) => ({ width: n, height: n });

  it("accepts a full set on an identical canvas", () => {
    expect(
      validateLayerDimensions({
        body: d(192),
        shadow: d(192),
        eyes_open: d(192),
        eyes_half: d(192),
        eyes_closed: d(192),
      }),
    ).toEqual({ width: 192, height: 192 });
  });

  it("rejects mismatched canvas dimensions with a clear error", () => {
    expect(() =>
      validateLayerDimensions({
        body: d(192),
        eyes_open: d(192),
        eyes_half: { width: 190, height: 192 },
        eyes_closed: d(192),
      }),
    ).toThrow(/registration mismatch.*eyes_half.*190x192.*192x192/s);
  });

  it("rejects a character missing a required layer", () => {
    expect(() =>
      validateLayerDimensions({ body: d(192), eyes_open: d(192), eyes_half: d(192) }),
    ).toThrow('missing required layer "eyes_closed"');
  });
});

describe("characterFromBytes", () => {
  const decodeStub = async () => ({}) as CanvasImageSource;
  const manifest = '{"name":"Test"}';

  it("loads a valid character", async () => {
    const c = await characterFromBytes(
      manifest,
      {
        body: pngHeader(64, 64),
        eyes_open: pngHeader(64, 64),
        eyes_half: pngHeader(64, 64),
        eyes_closed: pngHeader(64, 64),
      },
      decodeStub,
    );
    expect(c.name).toBe("Test");
    expect(c.width).toBe(64);
    expect(Object.keys(c.layers)).toHaveLength(4);
  });

  it("refuses to load on registration mismatch", async () => {
    await expect(
      characterFromBytes(
        manifest,
        {
          body: pngHeader(64, 64),
          eyes_open: pngHeader(64, 64),
          eyes_half: pngHeader(64, 64),
          eyes_closed: pngHeader(32, 32),
        },
        decodeStub,
      ),
    ).rejects.toThrow("registration mismatch");
  });

  it("rejects a bad manifest", () => {
    expect(() => parseManifest("{}")).toThrow('"name"');
    expect(() => parseManifest("not json")).toThrow("valid JSON");
  });
});

describe("placeholder character", () => {
  it("requires no external asset files — every layer is a data: URL", () => {
    const urls = Object.values(placeholderLayerUrls());
    expect(urls.length).toBeGreaterThanOrEqual(5);
    for (const url of urls) {
      expect(url.startsWith("data:image/svg+xml")).toBe(true);
    }
  });
});
