// Character asset contract (§8.2 of the brief).
// A character is a directory: character.json + PNG layers, all on an
// identical canvas registered to a common origin — compositing is
// drawImage(layer, 0, 0), no offset table.

export const LAYER_NAMES = [
  "body",
  "shadow",
  "eyes_open",
  "eyes_half",
  "eyes_closed",
  "mouth_closed",
  "mouth_open",
  "brows_neutral",
  "brows_happy",
  "brows_restless",
  "brows_flat",
] as const;

export type LayerName = (typeof LAYER_NAMES)[number];

export const REQUIRED_LAYERS: LayerName[] = [
  "body",
  "eyes_open",
  "eyes_half",
  "eyes_closed",
];

export interface CharacterManifest {
  name: string;
}

export interface Character {
  name: string;
  width: number;
  height: number;
  layers: Partial<Record<LayerName, CanvasImageSource>>;
}

/** Parse a PNG's IHDR to get its pixel dimensions. Throws on non-PNG bytes. */
export function pngDimensions(bytes: Uint8Array): { width: number; height: number } {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (bytes.length < 24 || sig.some((b, i) => bytes[i] !== b)) {
    throw new Error("not a PNG file");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

/**
 * Validate a character's layer set: required layers present, every layer on
 * an identical canvas. Returns the common dimensions. Rejects loudly — a
 * mis-registered character must never load (§8.2).
 */
export function validateLayerDimensions(
  layers: Partial<Record<LayerName, { width: number; height: number }>>,
): { width: number; height: number } {
  for (const name of REQUIRED_LAYERS) {
    if (!layers[name]) {
      throw new Error(`character is missing required layer "${name}"`);
    }
  }
  let common: { width: number; height: number } | null = null;
  let first: LayerName | null = null;
  for (const name of LAYER_NAMES) {
    const dim = layers[name];
    if (!dim) continue;
    if (!common) {
      common = { width: dim.width, height: dim.height };
      first = name;
    } else if (dim.width !== common.width || dim.height !== common.height) {
      throw new Error(
        `layer registration mismatch: "${name}" is ${dim.width}x${dim.height} ` +
          `but "${first}" is ${common.width}x${common.height}. All layers must ` +
          `export on an identical canvas.`,
      );
    }
  }
  if (!common) throw new Error("character has no layers");
  return common;
}

export function parseManifest(json: string): CharacterManifest {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    throw new Error("character.json is not valid JSON");
  }
  const name = (parsed as { name?: unknown }).name;
  if (typeof name !== "string" || name.length === 0) {
    throw new Error('character.json must have a non-empty "name" string');
  }
  return { name };
}

/**
 * Build a Character from raw PNG bytes per layer (the read_character_dir
 * path). Validates registration from IHDR before any decode.
 */
export async function characterFromBytes(
  manifestJson: string,
  layerBytes: Partial<Record<LayerName, Uint8Array>>,
  decode: (bytes: Uint8Array) => Promise<CanvasImageSource> = decodePng,
): Promise<Character> {
  const manifest = parseManifest(manifestJson);
  const dims: Partial<Record<LayerName, { width: number; height: number }>> = {};
  for (const name of LAYER_NAMES) {
    const bytes = layerBytes[name];
    if (bytes) dims[name] = pngDimensions(bytes);
  }
  const { width, height } = validateLayerDimensions(dims);
  const layers: Character["layers"] = {};
  for (const name of LAYER_NAMES) {
    const bytes = layerBytes[name];
    if (bytes) layers[name] = await decode(bytes);
  }
  return { name: manifest.name, width, height, layers };
}

async function decodePng(bytes: Uint8Array): Promise<CanvasImageSource> {
  return createImageBitmap(new Blob([bytes as BlobPart], { type: "image/png" }));
}

// ---------------------------------------------------------------------------
// Placeholder ball — drawn in code, zero external asset files (§8.1).
// SVG layers on a common 192x192 canvas (2x of the 96px display size).
// ---------------------------------------------------------------------------

const PLACEHOLDER_SIZE = 192;

const placeholderSvgs: Partial<Record<LayerName, string>> = {
  shadow: `<ellipse cx="96" cy="176" rx="56" ry="10" fill="black" opacity="1"/>`,
  body: `<defs><radialGradient id="g" cx="0.35" cy="0.3" r="0.9">
      <stop offset="0" stop-color="#7ec8e3"/><stop offset="0.7" stop-color="#2a6f8f"/>
      <stop offset="1" stop-color="#215a75"/></radialGradient></defs>
      <circle cx="96" cy="86" r="72" fill="url(#g)"/>`,
  eyes_open: `<ellipse cx="70" cy="76" rx="9" ry="13" fill="#0d2530"/>
      <ellipse cx="122" cy="76" rx="9" ry="13" fill="#0d2530"/>`,
  eyes_half: `<path d="M61 76 a9 13 0 0 0 18 0 z" fill="#0d2530"/>
      <path d="M113 76 a9 13 0 0 0 18 0 z" fill="#0d2530"/>`,
  eyes_closed: `<path d="M61 76 q9 6 18 0" stroke="#0d2530" stroke-width="3" fill="none" stroke-linecap="round"/>
      <path d="M113 76 q9 6 18 0" stroke="#0d2530" stroke-width="3" fill="none" stroke-linecap="round"/>`,
};

function svgDataUrl(inner: string): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${PLACEHOLDER_SIZE}" ` +
    `height="${PLACEHOLDER_SIZE}" viewBox="0 0 ${PLACEHOLDER_SIZE} ${PLACEHOLDER_SIZE}">${inner}</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("placeholder layer failed to decode"));
    img.src = url;
  });
}

/** Placeholder layer sources — all inline data: URLs (asserted in tests). */
export function placeholderLayerUrls(): Partial<Record<LayerName, string>> {
  const urls: Partial<Record<LayerName, string>> = {};
  for (const [name, inner] of Object.entries(placeholderSvgs)) {
    urls[name as LayerName] = svgDataUrl(inner);
  }
  return urls;
}

/** The built-in character. No file IO, no network. */
export async function placeholderCharacter(): Promise<Character> {
  const layers: Character["layers"] = {};
  for (const [name, url] of Object.entries(placeholderLayerUrls())) {
    layers[name as LayerName] = await loadImage(url);
  }
  return {
    name: "Placeholder Ball",
    width: PLACEHOLDER_SIZE,
    height: PLACEHOLDER_SIZE,
    layers,
  };
}
