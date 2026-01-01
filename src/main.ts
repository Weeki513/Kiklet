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

type TranscribeStatus = "idle" | "running" | "done" | "error";

type TranscribeState = {
  status: TranscribeStatus;
  text?: string;
  error?: string;
};

// In-memory storage for transcription state per recording ID
const transcriptionState = new Map<string, TranscribeState>();

type SettingsDto = {
  openaiApiKey: string;
  hasOpenaiApiKey: boolean;
  autoinsertEnabled: boolean;
};

const els = {
  status: document.getElementById("status") as HTMLDivElement,

  // Settings controls
  toggleAutostart: document.getElementById("toggle-autostart") as HTMLInputElement,
  autostartMsg: document.getElementById("autostart-msg") as HTMLDivElement,
  toggleAutoInsert: document.getElementById("toggle-autoinsert") as HTMLInputElement,
  autoinsertMsg: document.getElementById("autoinsert-msg") as HTMLDivElement,
  btnTestDeliver: document.getElementById("btn-test-deliver") as HTMLButtonElement,
  btnOpenPermissions: document.getElementById("btn-open-permissions") as HTMLButtonElement,
  deliverMsg: document.getElementById("deliver-msg") as HTMLDivElement,
  btnCheckPermissions: document.getElementById("btn-check-permissions") as HTMLButtonElement,
  permMsg: document.getElementById("perm-msg") as HTMLDivElement,
  selectTranslate: document.getElementById("select-translate") as HTMLSelectElement,
  fieldHotkey: document.getElementById("field-hotkey") as HTMLButtonElement,
  hotkeyValue: document.getElementById("hotkey-value") as HTMLSpanElement,
  hotkeyReset: document.getElementById("hotkey-reset") as HTMLButtonElement,
  hotkeyMsg: document.getElementById("hotkey-msg") as HTMLDivElement,
  fieldApiKey: document.getElementById("field-apikey") as HTMLButtonElement,
  apiKeyMasked: document.getElementById("apikey-masked") as HTMLSpanElement,
  selectModel: document.getElementById("select-model") as HTMLSelectElement,
  btnTutorial: document.getElementById("btn-tutorial") as HTMLButtonElement,

  // History
  historySub: document.getElementById("history-sub") as HTMLDivElement,
  btnStart: document.getElementById("btn-start") as HTMLButtonElement,
  btnStop: document.getElementById("btn-stop") as HTMLButtonElement,
  count: document.getElementById("count") as HTMLDivElement,
  items: document.getElementById("items") as HTMLDivElement,
  audio: document.getElementById("audio") as HTMLAudioElement,

  // API key modal
  modal: document.getElementById("modal") as HTMLDivElement,
  apiKey: document.getElementById("api-key") as HTMLInputElement,
  btnSaveKey: document.getElementById("btn-save-key") as HTMLButtonElement,
  btnCloseModal: document.getElementById("btn-close-modal") as HTMLButtonElement,
  modalStatus: document.getElementById("modal-status") as HTMLDivElement,
  modalError: document.getElementById("modal-error") as HTMLDivElement,

  // Details modal
  detailsModal: document.getElementById("details-modal") as HTMLDivElement,
  detailsSub: document.getElementById("details-sub") as HTMLDivElement,
  detailsText: document.getElementById("details-text") as HTMLTextAreaElement,
  btnDetailsTranscribe: document.getElementById("btn-details-transcribe") as HTMLButtonElement,
  btnDetailsCopy: document.getElementById("btn-details-copy") as HTMLButtonElement,
  btnDetailsReveal: document.getElementById("btn-details-reveal") as HTMLButtonElement,
  btnDetailsClose: document.getElementById("btn-details-close") as HTMLButtonElement,

  // Delete modal
  deleteModal: document.getElementById("delete-modal") as HTMLDivElement,
  deleteSub: document.getElementById("delete-sub") as HTMLDivElement,
  deleteError: document.getElementById("delete-error") as HTMLDivElement,
  btnDeleteCancel: document.getElementById("btn-delete-cancel") as HTMLButtonElement,
  btnDeleteConfirm: document.getElementById("btn-delete-confirm") as HTMLButtonElement,

  // Toast
  toast: document.getElementById("toast") as HTMLDivElement,
};

let isRecording = false;
let hasOpenaiApiKey = false;
let lastRecordings: RecordingItem[] = [];
let playingId: string | null = null;
let detailsTarget: RecordingItem | null = null;
let deleteTarget: RecordingItem | null = null;
let hotkeyCaptureActive = false;

let autostartBusy = false;
let autoinsertBusy = false;

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

function setStatus(text: string) {
  els.status.textContent = text;
}

function setRecordingState(next: boolean) {
  isRecording = next;
  setStatus(isRecording ? "Recording…" : "Idle");
  els.btnStart.disabled = isRecording;
  els.btnStop.disabled = !isRecording;
}

let toastTimer: number | null = null;
function showToast(text: string) {
  if (toastTimer) window.clearTimeout(toastTimer);
  els.toast.hidden = false;
  els.toast.textContent = text;
  toastTimer = window.setTimeout(() => {
    els.toast.hidden = true;
    els.toast.textContent = "";
    toastTimer = null;
  }, 1800);
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

function openApiKeyModal() {
  els.modal.removeAttribute("hidden");
  setModalError(null);
  setModalStatus(null);
  els.apiKey.value = "";
}

function closeApiKeyModal() {
  els.modal.setAttribute("hidden", "");
  setModalError(null);
  setModalStatus(null);
}

function openDetailsModal(item: RecordingItem) {
  detailsTarget = item;
  els.detailsSub.textContent = `${item.createdAt.replace("T", " ")} • ${fmtDuration(
    item.durationSec
  )} • ${fmtBytes(item.sizeBytes)}`;
  
  // Load transcription state if exists
  const state = transcriptionState.get(item.id);
  if (state?.status === "done" && state.text) {
    els.detailsText.value = state.text;
  } else {
    els.detailsText.value = "";
  }
  
  // Update UI based on transcription status
  updateDetailsTranscribeUI(state);
  
  els.detailsModal.removeAttribute("hidden");
  
  // Auto-transcribe if needed (once)
  void maybeAutoTranscribe(item, "details_open");
}

function updateDetailsTranscribeUI(state: TranscribeState | undefined) {
  if (!state || state.status === "idle") {
    els.btnDetailsTranscribe.disabled = false;
    els.btnDetailsTranscribe.textContent = "Transcribe";
    return;
  }
  
  if (state.status === "running") {
    els.btnDetailsTranscribe.disabled = true;
    els.btnDetailsTranscribe.textContent = "Transcribing…";
    return;
  }
  
  if (state.status === "done") {
    els.btnDetailsTranscribe.disabled = false;
    els.btnDetailsTranscribe.textContent = "Transcribe";
    return;
  }
  
  if (state.status === "error") {
    els.btnDetailsTranscribe.disabled = false;
    els.btnDetailsTranscribe.textContent = "Transcribe";
    // Error is shown in toast/UI elsewhere
    return;
  }
}

function closeDetailsModal() {
  els.detailsModal.setAttribute("hidden", "");
  detailsTarget = null;
}

function setDeleteError(text: string | null) {
  if (!text) {
    els.deleteError.hidden = true;
    els.deleteError.textContent = "";
    return;
  }
  els.deleteError.hidden = false;
  els.deleteError.textContent = text;
}

function openDeleteModal(item: RecordingItem) {
  deleteTarget = item;
  setDeleteError(null);
  els.deleteSub.textContent = `${item.createdAt.replace("T", " ")} • ${fmtDuration(
    item.durationSec
  )}`;
  els.deleteModal.removeAttribute("hidden");
}

function closeDeleteModal() {
  els.deleteModal.setAttribute("hidden", "");
  deleteTarget = null;
  setDeleteError(null);
}

function maskApiKey(key: string): string {
  const k = key.trim();
  if (!k) return "Не задан";
  if (k.length <= 10) return "sk‑••••••••";
  const prefix = k.startsWith("sk-") ? "sk-" : k.startsWith("sk-proj-") ? "sk-proj-" : k.slice(0, 3);
  const tail = k.slice(-3);
  return `${prefix}${"•".repeat(10)}${tail}`;
}

async function refreshSettings(allowPrompt: boolean) {
  const s = (await invoke("get_settings")) as SettingsDto;
  hasOpenaiApiKey = Boolean(s.hasOpenaiApiKey);
  els.apiKeyMasked.textContent = hasOpenaiApiKey ? maskApiKey(s.openaiApiKey ?? "") : "Не задан";
  els.toggleAutoInsert.checked = Boolean(s.autoinsertEnabled);
  if (allowPrompt && !hasOpenaiApiKey) openApiKeyModal();
}

async function refreshHistory() {
  const items = (await invoke("list_recordings")) as RecordingItem[];
  lastRecordings = items;
  els.count.textContent = `${items.length} записей`;
  els.historySub.textContent =
    items.length === 0 ? "Пока нет записей" : "Нажмите строку для действий";
  renderHistory(items);
}

function renderHistory(items: RecordingItem[]) {
  els.items.textContent = "";
  if (items.length === 0) {
    const empty = document.createElement("div");
    empty.className = "history-item";
    empty.innerHTML =
      '<div class="history-main"><div class="history-title">Нет записей</div><div class="meta">Нажмите Start, затем Stop</div></div><div class="history-actions"></div>';
    els.items.appendChild(empty);
    return;
  }

  for (const r of items) {
    const row = document.createElement("div");
    row.className = "history-item";

    const main = document.createElement("div");
    main.className = "history-main";
    const title = document.createElement("div");
    title.className = "history-title";
    title.title = r.createdAt.replace("T", " ");
    title.textContent = r.createdAt.replace("T", " ");
    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = `${fmtDuration(r.durationSec)} • ${fmtBytes(r.sizeBytes)}`;
    main.appendChild(title);
    main.appendChild(meta);

    const actions = document.createElement("div");
    actions.className = "history-actions";

    const playBtn = document.createElement("button");
    playBtn.type = "button";
    playBtn.className = "icon-action";
    const isPlaying = playingId === r.id && !els.audio.paused;
    playBtn.textContent = isPlaying ? "⏸" : "▶";
    if (isPlaying) playBtn.classList.add("playing");
    playBtn.title = isPlaying ? "Pause" : "Play";
    playBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      await togglePlay(r);
    });

    const detailsBtn = document.createElement("button");
    detailsBtn.type = "button";
    detailsBtn.className = "icon-action";
    detailsBtn.textContent = "≡";
    detailsBtn.title = "Details";
    detailsBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      openDetailsModal(r);
    });

    const delBtn = document.createElement("button");
    delBtn.type = "button";
    delBtn.className = "icon-action danger";
    delBtn.textContent = "⌫";
    delBtn.title = "Delete";
    delBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      openDeleteModal(r);
    });

    actions.appendChild(playBtn);
    actions.appendChild(detailsBtn);
    actions.appendChild(delBtn);

    row.appendChild(main);
    row.appendChild(actions);
    els.items.appendChild(row);
  }
}

async function togglePlay(item: RecordingItem) {
  try {
    if (playingId === item.id && !els.audio.paused) {
      els.audio.pause();
      return;
    }
    playingId = item.id;
    els.audio.src = convertFileSrc(item.path);
    await els.audio.play();
  } catch {
    // ignore
  } finally {
    renderHistory(lastRecordings);
  }
}

function loadUiPrefs() {
  els.selectTranslate.value = localStorage.getItem("kiklet.translate") ?? "off";
  els.selectModel.value = localStorage.getItem("kiklet.model") ?? "whisper-1";
  // hotkey is stored in settings.json (backend)
}

function saveUiPrefs() {
  localStorage.setItem("kiklet.translate", els.selectTranslate.value);
  localStorage.setItem("kiklet.model", els.selectModel.value);
}

function setAutostartMsg(text: string | null) {
  if (!text) {
    els.autostartMsg.hidden = true;
    els.autostartMsg.textContent = "";
    return;
  }
  els.autostartMsg.hidden = false;
  els.autostartMsg.textContent = text;
}

async function refreshAutostartStatus() {
  try {
    const enabled = (await invoke("get_autostart_status")) as boolean;
    els.toggleAutostart.checked = Boolean(enabled);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] get_autostart_status failed:", e);
    setAutostartMsg(`Error: ${msg}`);
    els.toggleAutostart.checked = false;
  }
}

async function onAutostartToggle() {
  if (autostartBusy) return;
  autostartBusy = true;
  setAutostartMsg("Saving…");
  const prev = !els.toggleAutostart.checked;

  els.toggleAutostart.disabled = true;
  try {
    const actual = (await invoke("set_autostart_enabled", {
      enabled: els.toggleAutostart.checked,
    })) as boolean;
    els.toggleAutostart.checked = Boolean(actual);
    setAutostartMsg("Saved");
    setTimeout(() => setAutostartMsg(null), 450);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] set_autostart_enabled failed:", e);
    els.toggleAutostart.checked = prev;
    setAutostartMsg(`Error: ${msg}`);
  } finally {
    els.toggleAutostart.disabled = false;
    autostartBusy = false;
  }
}

function setAutoinsertMsg(text: string | null) {
  if (!text) {
    els.autoinsertMsg.hidden = true;
    els.autoinsertMsg.textContent = "";
    return;
  }
  els.autoinsertMsg.hidden = false;
  els.autoinsertMsg.textContent = text;
}

function setDeliverMsg(text: string | null) {
  if (!text) {
    els.deliverMsg.hidden = true;
    els.deliverMsg.textContent = "";
    return;
  }
  els.deliverMsg.hidden = false;
  els.deliverMsg.textContent = text;
}

type PermissionCheckResult = {
  ok: boolean;
  needAccessibility: boolean;
};

function setPermMsg(text: string | null) {
  if (!text) {
    els.permMsg.hidden = true;
    els.permMsg.textContent = "";
    return;
  }
  els.permMsg.hidden = false;
  els.permMsg.textContent = text;
}

function permUiLabel(res: PermissionCheckResult): string {
  if (res.ok && !res.needAccessibility) return "OK";
  if (res.needAccessibility) return "Need Accessibility";
  return "Blocked";
}

function deliveryUiLabel(res: DeliveryResult): string {
  if (!res.ok) return res.detail ? `Failed: ${res.detail}` : "Failed";
  if (res.mode === "insert") return "Inserted";
  if (res.detail === "need_accessibility") return "Need Accessibility (Copied)";
  return "Copied";
}

async function onAutoInsertToggle() {
  if (autoinsertBusy) return;
  autoinsertBusy = true;
  const prev = !els.toggleAutoInsert.checked;

  els.toggleAutoInsert.disabled = true;
  setAutoinsertMsg("Saving…");
  try {
    const actual = (await invoke("set_autoinsert_enabled", {
      enabled: els.toggleAutoInsert.checked,
    })) as boolean;
    els.toggleAutoInsert.checked = Boolean(actual);
    setAutoinsertMsg("Saved");
    setTimeout(() => setAutoinsertMsg(null), 450);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] set_autoinsert_enabled failed:", e);
    els.toggleAutoInsert.checked = prev;
    setAutoinsertMsg(`Error: ${msg}`);
  } finally {
    els.toggleAutoInsert.disabled = false;
    autoinsertBusy = false;
  }
}

type DeliveryResult = {
  mode: "insert" | "copy";
  ok: boolean;
  detail?: string;
};

async function deliverAfterFinalText(text: string) {
  if (!els.toggleAutoInsert.checked) return;
  if (!text.trim()) return;
  try {
    const res = (await invoke("deliver_text", { text })) as DeliveryResult;
    setAutoinsertMsg(deliveryUiLabel(res));
    setDeliverMsg(deliveryUiLabel(res));
    setTimeout(() => {
      setAutoinsertMsg(null);
      setDeliverMsg(null);
    }, 1800);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] deliver_text failed:", e);
    setAutoinsertMsg(`Failed: ${msg}`);
    setDeliverMsg(`Failed: ${msg}`);
  }
}

async function maybeAutoTranscribe(
  record: RecordingItem,
  source: "stop" | "details_open"
): Promise<void> {
  // Check prerequisites
  if (!hasOpenaiApiKey) {
    console.log(`[kiklet] autoTranscribe skip source=${source} path=${record.path} reason=no_api_key`);
    return;
  }
  
  const model = els.selectModel.value.trim();
  if (!model) {
    console.log(`[kiklet] autoTranscribe skip source=${source} path=${record.path} reason=no_model`);
    return;
  }
  
  // Check current state
  const state = transcriptionState.get(record.id);
  if (state?.status === "running") {
    console.log(`[kiklet] autoTranscribe skip source=${source} path=${record.path} reason=already_running`);
    return;
  }
  
  if (state?.status === "done" && state.text) {
    console.log(`[kiklet] autoTranscribe skip source=${source} path=${record.path} reason=already_done`);
    return;
  }
  
  // Only auto-transcribe if status is idle or error (retry allowed)
  if (state && state.status !== "idle" && state.status !== "error") {
    console.log(`[kiklet] autoTranscribe skip source=${source} path=${record.path} reason=status=${state.status}`);
    return;
  }
  
  // Set status to running
  transcriptionState.set(record.id, { status: "running" });
  updateDetailsTranscribeUI(transcriptionState.get(record.id));
  
  console.log(`[kiklet] autoTranscribe start source=${source} path=${record.path}`);
  
  try {
    const text = (await invoke("transcribe_file", { path: record.path })) as string;
    const finalText = text?.trim() || "";
    
    // Update state to done
    transcriptionState.set(record.id, {
      status: "done",
      text: finalText,
    });
    
    console.log(`[kiklet] autoTranscribe ok source=${source} path=${record.path} len=${finalText.length}`);
    
    // Update UI if Details modal is open for this record
    if (detailsTarget?.id === record.id) {
      els.detailsText.value = finalText;
      updateDetailsTranscribeUI(transcriptionState.get(record.id));
    }
    
    // Try deliver if autoinsert is enabled (best-effort, don't fail on error)
    if (finalText) {
      try {
        await deliverAfterFinalText(finalText);
      } catch (e) {
        console.warn("[kiklet] autoTranscribe deliver failed (non-fatal):", e);
      }
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error(`[kiklet] autoTranscribe failed source=${source} path=${record.path} err=${msg}`);
    
    // Update state to error
    transcriptionState.set(record.id, {
      status: "error",
      error: msg,
    });
    
    // Update UI if Details modal is open
    if (detailsTarget?.id === record.id) {
      updateDetailsTranscribeUI(transcriptionState.get(record.id));
      showToast(`Transcribe error: ${msg}`);
    }
  }
}

function setHotkeyMsg(text: string | null) {
  if (!text) {
    els.hotkeyMsg.hidden = true;
    els.hotkeyMsg.textContent = "";
    return;
  }
  els.hotkeyMsg.hidden = false;
  els.hotkeyMsg.textContent = text;
}

async function refreshHotkey() {
  try {
    const hk = (await invoke("get_hotkey")) as string;
    els.hotkeyValue.textContent = hk;
    els.hotkeyReset.hidden = false;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] get_hotkey failed:", e);
    setHotkeyMsg(`Error: ${msg}`);
  }
}

function formatHotkey(e: KeyboardEvent): string {
  const parts: string[] = [];
  const isMac = navigator.platform.toLowerCase().includes("mac");
  if (isMac) {
    if (e.metaKey) parts.push("Cmd");
    if (e.altKey) parts.push("Option");
    if (e.shiftKey) parts.push("Shift");
    if (e.ctrlKey) parts.push("Ctrl");
  } else {
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.altKey) parts.push("Alt");
    if (e.shiftKey) parts.push("Shift");
    if (e.metaKey) parts.push("Win");
  }

  let key: string;
  if (e.code === "Space") key = "Space";
  else if (e.code === "Insert") key = "Ins";
  else if (/^F\d{1,2}$/i.test(e.key)) key = e.key.toUpperCase();
  else if (e.key.length === 1) key = e.key.toUpperCase();
  else key = e.key;

  // Disallow modifier-only.
  const modifierKeys = new Set(["Shift", "Control", "Alt", "Meta"]);
  if (modifierKeys.has(e.key)) return "";

  return [...parts, key].join("+");
}

function startHotkeyCapture() {
  if (hotkeyCaptureActive) return;
  hotkeyCaptureActive = true;
  setHotkeyMsg("Listening…");
  els.hotkeyValue.textContent = "Press shortcut…";

  const onKeyDown = (e: KeyboardEvent) => {
    e.preventDefault();
    if (e.key === "Escape") {
      hotkeyCaptureActive = false;
      window.removeEventListener("keydown", onKeyDown, true);
      setHotkeyMsg(null);
      void refreshHotkey();
      return;
    }
    hotkeyCaptureActive = false;
    window.removeEventListener("keydown", onKeyDown, true);

    const hk = formatHotkey(e);
    if (!hk) {
      setHotkeyMsg("Error: invalid shortcut");
      void refreshHotkey();
      return;
    }

    void (async () => {
      setHotkeyMsg("Saving…");
      els.fieldHotkey.disabled = true;
      els.hotkeyReset.disabled = true;
      try {
        const saved = (await invoke("set_hotkey", { accelerator: hk })) as string;
        els.hotkeyValue.textContent = saved;
        setHotkeyMsg("Saved");
        setTimeout(() => setHotkeyMsg(null), 450);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setHotkeyMsg(`Error: ${msg}`);
        console.error("[kiklet] set_hotkey failed:", err);
        void refreshHotkey();
      } finally {
        els.fieldHotkey.disabled = false;
        els.hotkeyReset.disabled = false;
      }
    })();
  };
  window.addEventListener("keydown", onKeyDown, true);
}

async function saveApiKey() {
  console.log("[kiklet] Save click");
  setModalError(null);
  setModalStatus(null);

  const candidate = els.apiKey.value.trim();
  console.log("[kiklet] api key len:", candidate.length);
  if (!candidate) {
    setModalError("Error: API key is empty.");
    return;
  }
  if (!(candidate.startsWith("sk-") || candidate.startsWith("sk-proj-"))) {
    setModalError("Error: API key must start with sk- (or sk-proj-).");
    return;
  }

  try {
    els.btnSaveKey.disabled = true;
    els.btnCloseModal.disabled = true;
    setModalStatus("Saving…");
    await invoke("set_openai_api_key", { apiKey: candidate });
    setModalStatus("Saved.");
    await new Promise((r) => setTimeout(r, 520));
    closeApiKeyModal();
    await refreshSettings(false);
    showToast("Сохранено");
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setModalError(`Error: ${msg}`);
    console.error("[kiklet] set_openai_api_key failed:", e);
  } finally {
    els.btnSaveKey.disabled = false;
    els.btnCloseModal.disabled = false;
  }
}

async function deleteRecording() {
  if (!deleteTarget) return;
  setDeleteError(null);
  try {
    await invoke("delete_recording", { path: deleteTarget.path });
    closeDeleteModal();
    showToast("Удалено");
    await refreshHistory();
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setDeleteError(`Error: ${msg}`);
    console.error("[kiklet] delete_recording failed:", e);
  }
}

async function start() {
  await invoke("start_recording");
}

async function stop() {
  try {
    const item = (await invoke("stop_recording")) as RecordingItem;
    // After recording stops, refresh history first, then maybe auto-transcribe
    await refreshHistory();
    // Use the item returned from stop_recording directly (it's already in lastRecordings after refreshHistory)
    const newRecord = lastRecordings.find((r) => r.id === item.id);
    if (newRecord) {
      await maybeAutoTranscribe(newRecord, "stop");
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] stop_recording failed:", e);
    showToast(`Error: ${msg}`);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  loadUiPrefs();

  els.toggleAutostart.addEventListener("change", onAutostartToggle);
  els.toggleAutoInsert.addEventListener("change", onAutoInsertToggle);
  els.selectTranslate.addEventListener("change", saveUiPrefs);
  els.selectModel.addEventListener("change", saveUiPrefs);

  els.btnTestDeliver.addEventListener("click", async () => {
    const text = `Kiklet test ${new Date().toISOString()}`;
    setDeliverMsg("Saving…");
    els.btnTestDeliver.disabled = true;
    try {
      const res = (await invoke("deliver_text", { text })) as DeliveryResult;
      setDeliverMsg(deliveryUiLabel(res));
      setTimeout(() => setDeliverMsg(null), 1800);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("[kiklet] deliver_text failed:", e);
      setDeliverMsg(`Failed: ${msg}`);
    } finally {
      els.btnTestDeliver.disabled = false;
    }
  });

  els.btnOpenPermissions.addEventListener("click", async () => {
    setDeliverMsg("Opening…");
    els.btnOpenPermissions.disabled = true;
    try {
      // Prefer native Accessibility prompt on macOS (best-effort).
      try {
        await invoke("request_accessibility");
      } catch (e) {
        console.warn("[kiklet] request_accessibility failed:", e);
      }
      await invoke("open_permissions_settings");
      setDeliverMsg("Opened");
      setTimeout(() => setDeliverMsg(null), 1800);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("[kiklet] open_permissions_settings failed:", e);
      setDeliverMsg(`Failed: ${msg}`);
    } finally {
      els.btnOpenPermissions.disabled = false;
    }
  });

  els.btnCheckPermissions.addEventListener("click", async () => {
    setPermMsg("Checking…");
    els.btnCheckPermissions.disabled = true;
    try {
      const res = (await invoke("check_permissions")) as PermissionCheckResult;
      setPermMsg(permUiLabel(res));
      setTimeout(() => setPermMsg(null), 4000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("[kiklet] check_permissions failed:", e);
      setPermMsg(`Failed: ${msg}`);
    } finally {
      els.btnCheckPermissions.disabled = false;
    }
  });

  els.fieldHotkey.addEventListener("click", startHotkeyCapture);
  els.hotkeyReset.addEventListener("click", () => {
    void (async () => {
      setHotkeyMsg("Saving…");
      els.fieldHotkey.disabled = true;
      els.hotkeyReset.disabled = true;
      try {
        const hk = (await invoke("reset_hotkey")) as string;
        els.hotkeyValue.textContent = hk;
        setHotkeyMsg("Saved");
        setTimeout(() => setHotkeyMsg(null), 450);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setHotkeyMsg(`Error: ${msg}`);
        console.error("[kiklet] reset_hotkey failed:", err);
        void refreshHotkey();
      } finally {
        els.fieldHotkey.disabled = false;
        els.hotkeyReset.disabled = false;
      }
    })();
  });

  els.fieldApiKey.addEventListener("click", openApiKeyModal);
  els.btnTutorial.addEventListener("click", () => showToast("Туториал скоро"));

  els.btnStart.addEventListener("click", start);
  els.btnStop.addEventListener("click", stop);

  els.btnSaveKey.addEventListener("click", saveApiKey);
  els.btnCloseModal.addEventListener("click", closeApiKeyModal);

  els.btnDetailsClose.addEventListener("click", closeDetailsModal);
  els.btnDetailsTranscribe.addEventListener("click", async () => {
    if (!detailsTarget) return;
    // Use the same maybeAutoTranscribe logic, but force it
    const state = transcriptionState.get(detailsTarget.id);
    if (state?.status === "running") {
      return; // Already running
    }
    // Reset to idle to allow retry
    transcriptionState.set(detailsTarget.id, { status: "idle" });
    await maybeAutoTranscribe(detailsTarget, "details_open");
  });
  els.btnDetailsCopy.addEventListener("click", async () => {
    const text = els.detailsText.value;
    if (!text.trim()) return;
    try {
      await navigator.clipboard.writeText(text);
      showToast("Copied");
    } catch {
      els.detailsText.focus();
      els.detailsText.select();
      showToast("Select and copy (Cmd+C)");
    }
  });
  els.btnDetailsReveal.addEventListener("click", async () => {
    if (!detailsTarget) return;
    await invoke("reveal_in_finder", { path: detailsTarget.path });
  });

  els.btnDeleteCancel.addEventListener("click", closeDeleteModal);
  els.btnDeleteConfirm.addEventListener("click", deleteRecording);

  els.audio.addEventListener("ended", () => {
    playingId = null;
    renderHistory(lastRecordings);
  });
  els.audio.addEventListener("pause", () => {
    renderHistory(lastRecordings);
  });
  els.audio.addEventListener("play", () => {
    renderHistory(lastRecordings);
  });

  setRecordingState(false);
  await refreshSettings(true);
  await refreshAutostartStatus();
  await refreshHotkey();
  await refreshHistory();

  await listen<boolean>("recording_state", (event) => {
    setRecordingState(Boolean(event.payload));
  });
  await listen("recordings_updated", async () => {
    await refreshHistory();
  });

  await listen("hotkey:toggle-record", async () => {
    try {
      if (isRecording) {
        await stop();
      } else {
        await start();
      }
    } catch (e) {
      console.error("[kiklet] hotkey toggle failed:", e);
    }
  });
});
