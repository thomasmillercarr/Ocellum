import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { placeholderCharacter } from "./character";
import { BlinkMachine, rollTransform } from "./behaviour";
import { renderFrame } from "./renderer";

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

// ---------------------------------------------------------------------------
// Bubble views
// ---------------------------------------------------------------------------

const chatLog = document.getElementById("chat-log") as HTMLDivElement;
const views = ["chat", "budget", "egress", "settings"] as const;
type View = (typeof views)[number];

function showView(view: View) {
  for (const v of views) {
    document.getElementById(`view-${v}`)!.hidden = v !== view;
  }
  document
    .querySelectorAll<HTMLButtonElement>("#bubble-tabs button")
    .forEach((b) => b.classList.toggle("active", b.dataset.view === view));
  if (view === "budget") void renderBudget();
  if (view === "egress") void renderEgress();
  if (view === "settings") void loadSettings();
  if (view === "chat") bubbleInput.focus();
}

document.querySelectorAll<HTMLButtonElement>("#bubble-tabs button").forEach((b) => {
  b.addEventListener("click", () => showView(b.dataset.view as View));
});

// --- Chat ---

function addMsg(cls: "user" | "assistant" | "error", text: string): HTMLDivElement {
  const div = document.createElement("div");
  div.className = `msg ${cls}`;
  div.textContent = text;
  chatLog.appendChild(div);
  chatLog.scrollTop = chatLog.scrollHeight;
  return div;
}

let streamingMsg: HTMLDivElement | null = null;

bubbleInput.addEventListener("keydown", (e) => {
  if (e.key !== "Enter" || bubbleInput.value.trim() === "") return;
  const text = bubbleInput.value.trim();
  bubbleInput.value = "";
  void invoke("report_input", { value: "" });
  addMsg("user", text);
  streamingMsg = addMsg("assistant", "");
  void invoke("chat_send", { text });
});

void listen<{ text: string }>("chat-delta", (e) => {
  if (!streamingMsg) streamingMsg = addMsg("assistant", "");
  streamingMsg.textContent += e.payload.text;
  chatLog.scrollTop = chatLog.scrollHeight;
});

void listen<{ text: string; tokens_in: number; tokens_out: number; cost_pence: number | null }>(
  "chat-done",
  (e) => {
    if (streamingMsg) {
      streamingMsg.textContent = e.payload.text;
      const meta = document.createElement("span");
      meta.className = "meta";
      const cost =
        e.payload.cost_pence != null ? ` · £${(e.payload.cost_pence / 100).toFixed(2)}` : "";
      meta.textContent = `${e.payload.tokens_in}→${e.payload.tokens_out} tok${cost}`;
      streamingMsg.appendChild(meta);
    }
    streamingMsg = null;
  },
);

void listen<{ message: string }>("chat-error", (e) => {
  streamingMsg?.remove();
  streamingMsg = null;
  addMsg("error", e.payload.message);
});

// --- Budget ---

async function renderBudget() {
  const el = document.getElementById("view-budget")!;
  const b = await invoke<Record<string, unknown>>("get_budget");
  if (b.mode === "api_key") {
    const month = b.month_pence as number;
    const cap = b.cap_pence as number;
    const pct = cap > 0 ? Math.min(100, (month / cap) * 100) : 0;
    const rows = (b.by_feature as { feature: string; pence: number; calls: number }[])
      .map((f) => `<tr><td>${f.feature}</td><td>${f.calls}</td><td>£${(f.pence / 100).toFixed(2)}</td></tr>`)
      .join("");
    el.innerHTML = `
      <div><strong>£${(month / 100).toFixed(2)}</strong> this month
        ${b.cap_enabled ? `of £${(cap / 100).toFixed(2)} cap` : "(no cap set)"}</div>
      <div class="budget-bar"><div style="width:${pct}%"></div></div>
      <table class="egress"><tr><th>Feature</th><th>Calls</th><th>Cost</th></tr>${rows}</table>`;
  } else if (b.mode === "claude_code") {
    el.innerHTML = `
      <div><strong>Ocellum's usage</strong> this month (via your Claude Code):</div>
      <div>${b.calls} calls · ${b.tokens_in} tokens in · ${b.tokens_out} tokens out</div>
      <div class="hint">${b.note}</div>`;
  } else {
    el.innerHTML = `<div>${b.calls} local calls this month. Ollama is free.</div>`;
  }
}

// --- Egress ---

async function renderEgress() {
  const el = document.getElementById("view-egress")!;
  const rows = await invoke<
    { id: number; destination: string; bytes: number; purpose: string; created_at: string }[]
  >("get_egress_log", { limit: 100 });
  if (rows.length === 0) {
    el.innerHTML = `<div class="hint">Nothing has left this machine yet.</div>`;
    return;
  }
  el.innerHTML = `<table class="egress">
    <tr><th>When</th><th>To</th><th>Bytes</th><th>Why</th></tr>
    ${rows
      .map(
        (r) =>
          `<tr><td>${r.created_at.slice(0, 19).replace("T", " ")}</td>
           <td>${r.destination}</td><td>${r.bytes}</td><td>${r.purpose}</td></tr>`,
      )
      .join("")}</table>`;
}

// --- Settings ---

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function loadSettings() {
  const s = await invoke<Record<string, unknown>>("get_settings");
  $<HTMLSelectElement>("set-provider").value = s.provider as string;
  $<HTMLInputElement>("set-anthropic-model").value = s.anthropic_model as string;
  $<HTMLInputElement>("set-openai-model").value = s.openai_model as string;
  $<HTMLInputElement>("set-ollama-model").value = s.ollama_model as string;
  $<HTMLInputElement>("set-ollama-url").value = s.ollama_url as string;
  $<HTMLInputElement>("set-cap-enabled").checked = s.cap_enabled === "1";
  $<HTMLInputElement>("set-cap-pounds").value = (
    parseFloat(s.cap_pence as string) / 100
  ).toString();
  $<HTMLInputElement>("set-anthropic-key").placeholder = s.has_anthropic_key
    ? "•••••••• (saved)"
    : "sk-ant-…";
  $<HTMLInputElement>("set-openai-key").placeholder = s.has_openai_key
    ? "•••••••• (saved)"
    : "sk-…";
  $<HTMLDivElement>("claude-code-status").textContent = s.claude_code_path
    ? `Detected: ${s.claude_code_path}`
    : "Claude Code not found on PATH.";
  updateProviderSections(s.provider as string);
}

function updateProviderSections(provider: string) {
  document
    .querySelectorAll<HTMLDivElement>("#view-settings [data-for]")
    .forEach((d) => (d.hidden = d.dataset.for !== provider));
}

function flashStatus(text: string) {
  $<HTMLDivElement>("settings-status").textContent = text;
  setTimeout(() => ($<HTMLDivElement>("settings-status").textContent = ""), 2000);
}

async function saveSetting(key: string, value: string) {
  await invoke("set_setting", { key, value });
  flashStatus("Saved ✓");
}

$<HTMLSelectElement>("set-provider").addEventListener("change", (e) => {
  const v = (e.target as HTMLSelectElement).value;
  updateProviderSections(v);
  void saveSetting("provider", v);
});
$<HTMLInputElement>("set-anthropic-model").addEventListener("change", (e) =>
  void saveSetting("anthropic_model", (e.target as HTMLInputElement).value),
);
$<HTMLInputElement>("set-openai-model").addEventListener("change", (e) =>
  void saveSetting("openai_model", (e.target as HTMLInputElement).value),
);
$<HTMLInputElement>("set-ollama-model").addEventListener("change", (e) =>
  void saveSetting("ollama_model", (e.target as HTMLInputElement).value),
);
$<HTMLInputElement>("set-ollama-url").addEventListener("change", (e) =>
  void saveSetting("ollama_url", (e.target as HTMLInputElement).value),
);
$<HTMLInputElement>("set-cap-enabled").addEventListener("change", (e) =>
  void saveSetting("cap_enabled", (e.target as HTMLInputElement).checked ? "1" : "0"),
);
$<HTMLInputElement>("set-cap-pounds").addEventListener("change", (e) => {
  const pounds = parseFloat((e.target as HTMLInputElement).value) || 0;
  void saveSetting("cap_pence", String(Math.round(pounds * 100)));
});

for (const p of ["anthropic", "openai"] as const) {
  $<HTMLInputElement>(`set-${p}-key`).addEventListener("change", async (e) => {
    const key = (e.target as HTMLInputElement).value.trim();
    if (!key) return;
    await invoke("set_api_key", { provider: p, key });
    (e.target as HTMLInputElement).value = "";
    (e.target as HTMLInputElement).placeholder = "•••••••• (saved)";
    flashStatus("Key stored in Windows Credential Manager ✓");
  });
}

void listen("toggle-bubble", () => void setBubbleOpen(bubble.hidden));
void listen("open-bubble", () => void setBubbleOpen(true));
void listen("close-bubble", () => void setBubbleOpen(false));

window.addEventListener("resize", () => void reportHitRegions());

void reportHitRegions();

async function startCharacter() {
  const character = await placeholderCharacter();
  const canvas = document.getElementById("pet-canvas") as HTMLCanvasElement;
  canvas.width = character.width;
  canvas.height = character.height;
  const ctx = canvas.getContext("2d")!;
  const blink = new BlinkMachine();
  // ponytail: radius convention is 0.375·canvas width (matches the
  // placeholder's 72px ball on a 192px canvas); make it a manifest field if
  // a real character ever needs a different pivot.
  const radius = character.width * 0.375;
  const t0 = performance.now();
  const frame = (now: number) => {
    const t = now - t0;
    renderFrame(ctx, character, blink.at(t), rollTransform(t, radius));
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

void startCharacter();
