import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const pet = document.getElementById("pet") as HTMLDivElement;
const bubble = document.getElementById("bubble") as HTMLDivElement;
const bubbleClose = document.getElementById("bubble-close") as HTMLButtonElement;
const bubbleInput = document.getElementById("bubble-input") as HTMLInputElement;

// Hit regions are reported to Rust in logical (CSS-pixel) window coordinates.
// The Rust poll loop converts them to physical screen coords each tick, so
// window moves and DPI changes need no re-report.
interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function elementRect(el: HTMLElement): Rect {
  const r = el.getBoundingClientRect();
  return { x: r.left, y: r.top, w: r.width, h: r.height };
}

async function reportHitRegions() {
  const rects: Rect[] = [elementRect(pet)];
  if (!bubble.hidden) rects.push(elementRect(bubble));
  await invoke("set_hit_regions", { rects });
}

async function setBubbleOpen(open: boolean) {
  bubble.hidden = !open;
  await reportHitRegions();
  await invoke("bubble_state", { open });
  if (open) bubbleInput.focus();
}

pet.addEventListener("click", () => void setBubbleOpen(bubble.hidden));
bubbleClose.addEventListener("click", () => void setBubbleOpen(false));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !bubble.hidden) void setBubbleOpen(false);
});

// Mirror input value to Rust so the test control channel can read it.
bubbleInput.addEventListener("input", () => {
  void invoke("report_input", { value: bubbleInput.value });
});

void listen("toggle-bubble", () => void setBubbleOpen(bubble.hidden));
void listen("open-bubble", () => void setBubbleOpen(true));
void listen("close-bubble", () => void setBubbleOpen(false));

window.addEventListener("resize", () => void reportHitRegions());

void reportHitRegions();
