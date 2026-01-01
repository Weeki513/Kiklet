import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type RecordingItem = {
  id: string;
  filename: string;
  createdAt: string;
  durationSec: number;
  sizeBytes: number;
  path: string;
};

type SettingsDto = {
  openaiApiKey: string;
  hasOpenaiApiKey: boolean;
};

const els = {
  status: document.getElementById("status") as HTMLDivElement,
  btnStart: document.getElementById("btn-start") as HTMLButtonElement,
  btnStop: document.getElementById("btn-stop") as HTMLButtonElement,
  btnTranscribe: document.getElementById("btn-transcribe") as HTMLButtonElement,
  btnCopy: document.getElementById("btn-copy") as HTMLButtonElement,
  btnFolder: document.getElementById("btn-folder") as HTMLButtonElement,
  btnSettings: document.getElementById("btn-settings") as HTMLButtonElement,
  items: document.getElementById("items") as HTMLDivElement,
  count: document.getElementById("count") as HTMLDivElement,
  audio: document.getElementById("audio") as HTMLAudioElement,
  panelHint: document.getElementById("panel-hint") as HTMLDivElement,
  result: document.getElementById("result") as HTMLTextAreaElement,
  modal: document.getElementById("modal") as HTMLDivElement,
  apiKey: document.getElementById("api-key") as HTMLInputElement,
  btnSaveKey: document.getElementById("btn-save-key") as HTMLButtonElement,
  btnCloseModal: document.getElementById("btn-close-modal") as HTMLButtonElement,
  modalStatus: document.getElementById("modal-status") as HTMLDivElement,
  modalError: document.getElementById("modal-error") as HTMLDivElement,
};

let isRecording = false;
let isTranscribing = false;
let hasOpenaiApiKey = false;
let selectedRecording: RecordingItem | null = null;
let lastRecordings: RecordingItem[] = [];

function fmtDuration(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "0s";
  if (sec < 60) return `${sec.toFixed(1)}s`;
  const m = Math.floor(sec / 60);
  const s = Math.round(sec - m * 60);
  return `${m}m ${s}s`;
}

function fmtBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"] as const;
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  const dp = u === 0 ? 0 : u === 1 ? 1 : 2;
  return `${v.toFixed(dp)} ${units[u]}`;
}

function setRecordingState(next: boolean) {
  isRecording = next;
  els.status.textContent = isRecording ? "Recording…" : isTranscribing ? "Transcribing…" : "Idle";
  els.btnStart.disabled = isRecording;
  els.btnStop.disabled = !isRecording;
}

function setTranscribingState(next: boolean) {
  isTranscribing = next;
  els.status.textContent = isRecording ? "Recording…" : isTranscribing ? "Transcribing…" : "Idle";
  els.btnTranscribe.disabled =
    isTranscribing || isRecording || !hasOpenaiApiKey || !getTranscribeTarget();
  els.btnCopy.disabled = isTranscribing || els.result.value.trim().length === 0;
}

function itemSub(r: RecordingItem): string {
  return `${fmtDuration(r.durationSec)} • ${fmtBytes(r.sizeBytes)}`;
}

function setPanelHint(text: string) {
  els.panelHint.textContent = text;
}

function setModalError(text: string | null) {
  if (!text) {
    els.modalError.hidden = true;
    els.modalError.textContent = "";
    return;
  }
  els.modalError.hidden = false;
  els.modalError.textContent = text;
}

function setModalStatus(text: string | null) {
  if (!text) {
    els.modalStatus.hidden = true;
    els.modalStatus.textContent = "";
    return;
  }
  els.modalStatus.hidden = false;
  els.modalStatus.textContent = text;
}

function openApiKeyModal(reason?: string) {
  els.modal.removeAttribute("hidden");
  setModalError(null);
  setModalStatus(null);
  if (reason) setPanelHint(reason);
}

function closeApiKeyModal() {
  els.modal.setAttribute("hidden", "");
  console.log("[kiklet] modal hidden=", els.modal.hasAttribute("hidden"));
  setModalError(null);
  setModalStatus(null);
}

function renderItems(items: RecordingItem[]) {
  lastRecordings = items;
  if (selectedRecording) {
    const stillExists = items.some((r) => r.id === selectedRecording?.id);
    if (!stillExists) selectedRecording = null;
  }

  els.count.textContent = `${items.length}`;
  els.items.textContent = "";

  if (items.length === 0) {
    const empty = document.createElement("div");
    empty.className = "item";
    empty.innerHTML =
      '<div class="item-main"><div class="item-title">No recordings yet</div><div class="item-sub">Use tray menu or Cmd+Shift+Space</div></div><div class="item-actions"></div>';
    els.items.appendChild(empty);
    return;
  }

  for (const r of items) {
    const row = document.createElement("div");
    row.className = "item";
    if (selectedRecording?.id === r.id) {
      row.style.background = "rgba(255,255,255,0.06)";
    }
    row.addEventListener("click", () => {
      selectedRecording = r;
      els.btnTranscribe.disabled = isTranscribing || isRecording || !getTranscribeTarget();
      renderItems(lastRecordings);
    });

    const main = document.createElement("div");
    main.className = "item-main";

    const title = document.createElement("div");
    title.className = "item-title";
    title.textContent = r.createdAt.replace("T", " ");

    const sub = document.createElement("div");
    sub.className = "item-sub";
    sub.textContent = itemSub(r);

    main.appendChild(title);
    main.appendChild(sub);

    const actions = document.createElement("div");
    actions.className = "item-actions";

    const btnPlay = document.createElement("button");
    btnPlay.className = "linkbtn";
    btnPlay.type = "button";
    btnPlay.textContent = "Play";
    btnPlay.addEventListener("click", async () => {
      const src = convertFileSrc(r.path);
      els.audio.src = src;
      try {
        await els.audio.play();
      } catch {
        // ignore autoplay restrictions; user can press play manually
      }
    });

    const btnReveal = document.createElement("button");
    btnReveal.className = "linkbtn";
    btnReveal.type = "button";
    btnReveal.textContent = "Reveal";
    btnReveal.addEventListener("click", async () => {
      await invoke("reveal_in_finder", { path: r.path });
    });

    actions.appendChild(btnPlay);
    actions.appendChild(btnReveal);

    row.appendChild(main);
    row.appendChild(actions);
    els.items.appendChild(row);
  }
}

async function refresh() {
  const items = (await invoke("list_recordings")) as RecordingItem[];
  renderItems(items);
  els.btnTranscribe.disabled =
    isTranscribing || isRecording || !hasOpenaiApiKey || !getTranscribeTarget();
}

async function start() {
  await invoke("start_recording");
}

async function stop() {
  await invoke("stop_recording");
}

async function openFolder() {
  await invoke("open_recordings_folder");
}

function getTranscribeTarget(): RecordingItem | null {
  if (selectedRecording) return selectedRecording;
  if (lastRecordings.length > 0) return lastRecordings[0];
  return null;
}

async function loadSettingsAndMaybePrompt() {
  const s = (await invoke("get_settings")) as SettingsDto;
  hasOpenaiApiKey = Boolean(s.hasOpenaiApiKey);
  els.apiKey.value = s.openaiApiKey ?? "";
  els.btnTranscribe.disabled =
    isTranscribing || isRecording || !hasOpenaiApiKey || !getTranscribeTarget();

  if (!hasOpenaiApiKey) {
    openApiKeyModal("Set your OpenAI API key to transcribe.");
  } else {
    setPanelHint("");
  }
}

async function saveApiKey() {
  console.log("[kiklet] Save click");
  setModalError(null);
  setModalStatus(null);

  const candidate = els.apiKey.value.trim();
  console.log("[kiklet] api key len:", candidate.length);
  if (!candidate) {
    setModalError("Error: API key is empty.");
    console.error("[kiklet] Save click: empty API key");
    return;
  }
  if (!(candidate.startsWith("sk-") || candidate.startsWith("sk-proj-"))) {
    setModalError("Error: API key must start with sk- (or sk-proj-).");
    console.error("[kiklet] Save click: invalid apiKey prefix");
    return;
  }

  try {
    els.btnSaveKey.disabled = true;
    els.btnCloseModal.disabled = true;
    setModalStatus("Saving…");

    // Deterministic mapping: Tauri default arg case is camelCase, so Rust `api_key` expects `apiKey`.
    await invoke("set_openai_api_key", { apiKey: candidate });

    setModalStatus("Saved.");
    await new Promise((r) => setTimeout(r, 450));
    console.log("[kiklet] closing modal");
    closeApiKeyModal();
    await loadSettingsAndMaybePrompt();
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setModalError(`Error: ${msg}`);
    console.error("[kiklet] set_openai_api_key failed:", e);
  }
  finally {
    els.btnSaveKey.disabled = false;
    els.btnCloseModal.disabled = false;
  }
}

function wireApiKeyModal() {
  if (els.modal.dataset.wired === "1") return;
  els.modal.dataset.wired = "1";
  console.log("[kiklet] wireApiKeyModal ok");

  // Ensure we don't accidentally double-wire on HMR: replace nodes to drop any prior listeners.
  const saveClone = els.btnSaveKey.cloneNode(true) as HTMLButtonElement;
  els.btnSaveKey.replaceWith(saveClone);
  els.btnSaveKey = saveClone;

  const closeClone = els.btnCloseModal.cloneNode(true) as HTMLButtonElement;
  els.btnCloseModal.replaceWith(closeClone);
  els.btnCloseModal = closeClone;

  const settingsClone = els.btnSettings.cloneNode(true) as HTMLButtonElement;
  els.btnSettings.replaceWith(settingsClone);
  els.btnSettings = settingsClone;

  els.btnSaveKey.addEventListener("click", saveApiKey);
  els.btnCloseModal.addEventListener("click", () => {
    console.log("[kiklet] close click");
    closeApiKeyModal();
  });
  els.btnSettings.addEventListener("click", () => {
    openApiKeyModal();
  });
}

async function transcribe() {
  setPanelHint("");
  const target = getTranscribeTarget();
  if (!target) {
    setPanelHint("No recording selected.");
    return;
  }

  setTranscribingState(true);
  try {
    const text = (await invoke("transcribe_file", {
      path: target.path,
    })) as string;
    els.result.value = text ?? "";
    setPanelHint(`Done: ${target.createdAt.replace("T", " ")}`);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setPanelHint(msg);
  } finally {
    setTranscribingState(false);
  }
}

async function copyResult() {
  const text = els.result.value;
  if (!text.trim()) return;
  try {
    await navigator.clipboard.writeText(text);
    setPanelHint("Copied.");
  } catch {
    els.result.focus();
    els.result.select();
    setPanelHint("Select and copy (Cmd+C).");
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  els.btnStart.addEventListener("click", start);
  els.btnStop.addEventListener("click", stop);
  els.btnTranscribe.addEventListener("click", transcribe);
  els.btnCopy.addEventListener("click", copyResult);
  els.btnFolder.addEventListener("click", openFolder);
  wireApiKeyModal();

  setRecordingState(false);
  setTranscribingState(false);
  els.result.value = "";
  setPanelHint("");
  await refresh();
  await loadSettingsAndMaybePrompt();

  await listen<boolean>("recording_state", (event) => {
    setRecordingState(Boolean(event.payload));
  });
  await listen("recordings_updated", async () => {
    await refresh();
  });
});
