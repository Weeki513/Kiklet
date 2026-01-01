import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";

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
  hotkeyAccelerator: string;
  hotkeyCode?: string;
  hotkeyMods?: { cmd: boolean; ctrl: boolean; alt: boolean; shift: boolean };
  pttEnabled: boolean;
  pttThresholdMs: number;
  translateTarget?: string | null;
  translateModel: string;
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
  inputTranslate: document.getElementById("input-translate") as HTMLInputElement,
  btnTranslateClear: document.getElementById("btn-translate-clear") as HTMLButtonElement,
  translateAutocomplete: document.getElementById("translate-autocomplete") as HTMLDivElement,
  selectTranslateModel: document.getElementById("select-translate-model") as HTMLSelectElement,
  fieldHotkey: document.getElementById("field-hotkey") as HTMLButtonElement,
  hotkeyValue: document.getElementById("hotkey-value") as HTMLSpanElement,
  hotkeyReset: document.getElementById("hotkey-reset") as HTMLButtonElement,
  hotkeyMsg: document.getElementById("hotkey-msg") as HTMLDivElement,
  inputPttThreshold: document.getElementById("input-ptt-threshold") as HTMLInputElement,
  pttThresholdMsg: document.getElementById("ptt-threshold-msg") as HTMLDivElement,
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
  btnClearHistory: document.getElementById("btn-clear-history") as HTMLButtonElement,
  clearStatus: document.getElementById("clear-status") as HTMLDivElement,
  jsError: document.getElementById("js-error") as HTMLDivElement,
  btnDebugStorage: document.getElementById("btn-debug-storage") as HTMLButtonElement,
  debugStorageOut: document.getElementById("debug-storage-out") as HTMLPreElement,

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

// PTT state machine
let isHotkeyHeld = false;
let pttArmed = false;
let pttActive = false;
let holdMode = false; // true when hold > 300ms, false for tap
let pttTimer: number | null = null;
let pttThresholdMs = 300;

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

function setClearStatus(text: string) {
  els.clearStatus.hidden = false;
  els.clearStatus.textContent = text;
}

function setJsError(text: string) {
  els.jsError.hidden = false;
  els.jsError.textContent = text;
}

function formatAnyError(e: unknown): string {
  if (e instanceof Error) return e.stack || e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
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

// Popular languages for autocomplete
const POPULAR_LANGUAGES = [
  "English", "Русский", "Español", "Français", "Deutsch", "Português", "Italiano",
  "中文", "日本語", "한국어", "العربية", "हिन्दी", "Türkçe", "Polski", "Nederlands",
  "Svenska", "Norsk", "Dansk", "Suomi", "Čeština", "Română", "Magyar", "Ελληνικά",
  "Български", "Українська", "Tiếng Việt", "ไทย", "Bahasa Indonesia", "Bahasa Melayu",
  "עברית", "فارسی", "اردو", "বাংলা", "தமிழ்", "తెలుగు", "മലയാളം", "ಕನ್ನಡ",
  "ગુજરાતી", "ਪੰਜਾਬੀ", "मराठी", "O'zbek", "Қазақ", "Azərbaycan", "ქართული",
  "Հայերեն", "Shqip", "Hrvatski", "Srpski", "Slovenčina", "Slovenščina", "Eesti",
  "Latviešu", "Lietuvių", "Македонски", "Bosanski", "Монгол", "नेपाली", "සිංහල",
  "မြန်မာ", "ខ្មែរ", "Lao", "Cebuano", "Tagalog", "Hausa", "Yorùbá", "Igbo",
  "Zulu", "Afrikaans", "Swahili", "Amharic", "Somali", "Kinyarwanda", "Luganda",
  "Kiswahili", "Xhosa", "Tswana", "Sesotho", "Malagasy", "Maltese", "Icelandic",
  "Irish", "Welsh", "Breton", "Basque", "Catalan", "Galician", "Luxembourgish",
  "Faroese", "Kurdish", "Pashto", "Dari", "Tajik", "Kyrgyz", "Turkmen", "Mongolian",
  "Tibetan", "Nepali", "Sinhala", "Dhivehi", "Maldivian", "Chichewa"
];

function filterLanguages(query: string): string[] {
  if (!query || query.trim() === "") return [];
  const q = query.toLowerCase();
  return POPULAR_LANGUAGES.filter((lang) => lang.toLowerCase().includes(q));
}

function showAutocomplete(query: string) {
  const matches = filterLanguages(query);
  const autocomplete = els.translateAutocomplete;
  autocomplete.innerHTML = "";
  
  if (matches.length === 0) {
    autocomplete.style.display = "none";
    return;
  }
  
  for (const lang of matches.slice(0, 10)) {
    const item = document.createElement("div");
    item.style.cssText = "padding: 8px 12px; cursor: pointer; border-bottom: 1px solid #eee;";
    item.textContent = lang;
    item.addEventListener("mouseenter", () => {
      item.style.backgroundColor = "#f5f5f5";
    });
    item.addEventListener("mouseleave", () => {
      item.style.backgroundColor = "white";
    });
    item.addEventListener("click", () => {
      els.inputTranslate.value = lang;
      autocomplete.style.display = "none";
      els.btnTranslateClear.style.display = "block";
      saveTranslateTarget();
    });
    autocomplete.appendChild(item);
  }
  
  autocomplete.style.display = "block";
}

async function saveTranslateTarget() {
  const value = els.inputTranslate.value.trim();
  const target = value === "" || value === "Не переводить" ? null : value;
  try {
    await invoke("set_translate_target", { target });
    await refreshSettings(false);
  } catch (e) {
    console.error("[kiklet] set_translate_target failed:", e);
  }
}

async function loadTranslateModels(): Promise<void> {
  try {
    const models = (await invoke("list_models")) as string[];
    if (models.length > 0) {
      // Clear existing options except the first one (if it's a placeholder)
      const select = els.selectTranslateModel;
      const currentValue = select.value;
      select.innerHTML = "";
      
      // Add models
      for (const model of models) {
        const option = document.createElement("option");
        option.value = model;
        option.textContent = model;
        select.appendChild(option);
      }
      
      // Restore previous selection if it exists
      if (currentValue && Array.from(select.options).some((opt) => opt.value === currentValue)) {
        select.value = currentValue;
      } else if (models.includes("gpt-4o")) {
        select.value = "gpt-4o";
      } else if (models.length > 0) {
        select.value = models[0];
      }
      
      console.log(`[kiklet][ui] loaded ${models.length} translate models`);
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.warn(`[kiklet][ui] loadTranslateModels failed: ${msg}`);
  }
}

async function refreshSettings(allowPrompt: boolean) {
  const s = (await invoke("get_settings")) as SettingsDto;
  hasOpenaiApiKey = Boolean(s.hasOpenaiApiKey);
  els.apiKeyMasked.textContent = hasOpenaiApiKey ? maskApiKey(s.openaiApiKey ?? "") : "Не задан";
  els.toggleAutoInsert.checked = Boolean(s.autoinsertEnabled);
  pttThresholdMs = s.pttThresholdMs ?? 300;
  els.inputPttThreshold.value = String(s.pttThresholdMs ?? 300);
  
  // Update translate settings
  if (s.translateTarget && s.translateTarget.trim() !== "") {
    els.inputTranslate.value = s.translateTarget;
    els.btnTranslateClear.style.display = "block";
  } else {
    els.inputTranslate.value = "Не переводить";
    els.btnTranslateClear.style.display = "none";
  }
  els.selectTranslateModel.value = s.translateModel || "gpt-4o";
  
  // Load models list if API key is available
  if (hasOpenaiApiKey) {
    loadTranslateModels().catch((e: unknown) => {
      console.warn("[kiklet][ui] loadTranslateModels failed:", e);
    });
  }
  
  // Start PTT tracker if enabled (schedule after UI is ready, not immediately)
  // ptt_enabled is derived from ptt_threshold_ms > 0
  const pttEnabled = (s.pttThresholdMs ?? 0) > 0;
  console.log("[kiklet][ptt][ui] check ptt_threshold_ms=", s.pttThresholdMs, "ptt_enabled=", pttEnabled, "hotkeyCode=", s.hotkeyCode);
  if (pttEnabled && s.hotkeyCode && s.hotkeyMods) {
    console.log("[kiklet][ptt][ui] schedule start");
    // Schedule PTT start after UI is ready (not on main thread during startup)
    setTimeout(async () => {
      try {
        console.log("[kiklet][ptt][ui] starting PTT tracker");
        const started = await invoke("ptt_start", {
          code: s.hotkeyCode,
          mods: s.hotkeyMods,
          accelerator: s.hotkeyAccelerator || "",
        }) as boolean;
        if (started) {
          console.log("[kiklet][ptt][ui] start ok");
        } else {
          console.warn("[kiklet][ptt][ui] start failed (returned false)");
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.includes("need_accessibility")) {
          console.warn("[kiklet][ptt][ui] start failed err=need_accessibility");
        } else {
          console.error("[kiklet][ptt][ui] start failed err=", e);
        }
      }
    }, 0);
  } else {
    // Stop PTT if disabled
    console.log("[kiklet][ptt][ui] PTT disabled, stopping");
    setTimeout(async () => {
      try {
        await invoke("ptt_stop");
      } catch (e) {
        console.error("[kiklet][ptt][ui] PTT stop error:", e);
      }
    }, 0);
  }
  
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
  // translate is stored in settings.json (backend)
  els.selectModel.value = localStorage.getItem("kiklet.model") ?? "whisper-1";
  // hotkey is stored in settings.json (backend)
}

function saveUiPrefs() {
  // Translate settings saved via change handlers
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
  // Always call deliver_text - it will handle autoinsert_enabled internally
  // Even if autoinsert is disabled, text should go to clipboard
  if (!text.trim()) return;
  try {
    const res = (await invoke("deliver_text", { text })) as DeliveryResult;
    const delivered = res.mode === "insert" ? "pasted" : "clipboard";
    const reason = res.detail || "";
    console.log(`[kiklet][ui] deliver result=${delivered} reason=${reason}`);
    // Only show UI message if autoinsert is enabled (to avoid noise when disabled)
    if (els.toggleAutoInsert.checked) {
      setAutoinsertMsg(deliveryUiLabel(res));
      setDeliverMsg(deliveryUiLabel(res));
      setTimeout(() => {
        setAutoinsertMsg(null);
        setDeliverMsg(null);
      }, 1800);
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("[kiklet] deliver_text failed:", e);
    if (els.toggleAutoInsert.checked) {
      setAutoinsertMsg(`Failed: ${msg}`);
      setDeliverMsg(`Failed: ${msg}`);
    }
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
    
    // Try translate + deliver if autoinsert is enabled (best-effort, don't fail on error)
    if (finalText) {
      try {
        let textToDeliver = finalText;
        
        // Check if translation is enabled
        const settings = (await invoke("get_settings")) as SettingsDto;
        const shouldTranslate = settings.translateTarget 
          && settings.translateTarget.trim() !== "" 
          && settings.translateTarget.trim().toLowerCase() !== "не переводить";
        console.log(`[kiklet][translate] target=${settings.translateTarget || "none"} model=${settings.translateModel || "gpt-4o"} skip=${!shouldTranslate}`);
        
        if (shouldTranslate) {
          try {
            console.log(`[kiklet][ui] translate start lang=${settings.translateTarget} model=${settings.translateModel}`);
            textToDeliver = (await invoke("translate_text", {
              text: finalText,
              targetLanguage: settings.translateTarget!,
              model: settings.translateModel || "gpt-4o",
            })) as string;
            console.log(`[kiklet][ui] translate ok len=${textToDeliver.length}`);
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(`[kiklet][ui] translate fail err=${msg}, using original text`);
            // Fallback to original text on translation error
            textToDeliver = finalText;
          }
        }
        
        await deliverAfterFinalText(textToDeliver);
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

function setPttThresholdMsg(msg: string | null) {
  if (msg) {
    els.pttThresholdMsg.textContent = msg;
    els.pttThresholdMsg.hidden = false;
  } else {
    els.pttThresholdMsg.hidden = true;
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

// Check if event is a modifier key
function isModifierKey(e: KeyboardEvent): boolean {
  const modifierCodes = new Set([
    "ShiftLeft", "ShiftRight",
    "ControlLeft", "ControlRight",
    "AltLeft", "AltRight",
    "MetaLeft", "MetaRight",
  ]);
  const modifierKeys = new Set([
    "Shift", "Control", "Alt", "Meta", "Command", "Option",
  ]);
  return modifierCodes.has(e.code) || modifierKeys.has(e.key);
}

// Normalize code - keep full code for backend (e.g. "KeyS", "Digit1", "ArrowLeft")
function normalizeCode(code: string): string {
  // Keep the full code as-is - backend will map it to plugin candidates
  // This ensures we don't lose information (e.g. "KeyS" vs "S")
  return code;
}

// Format modifier-only hotkey
function formatModifierOnly(e: KeyboardEvent): string | null {
  const isMac = navigator.platform.toLowerCase().includes("mac");
  if (isMac) {
    if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) return "Cmd";
    if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey) return "Option";
    if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) return "Shift";
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) return "Ctrl";
  } else {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return "Ctrl";
    if (e.altKey && !e.ctrlKey && !e.shiftKey && !e.metaKey) return "Alt";
    if (e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) return "Shift";
    if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) return "Win";
  }
  return null;
}

function startHotkeyCapture() {
  if (hotkeyCaptureActive) return;
  hotkeyCaptureActive = true;
  setHotkeyMsg("Listening…");
  els.hotkeyValue.textContent = "Press shortcut…";

  let pendingModifier: string | null = null;
  let hadNonModifier = false;

  const onKeyDown = (e: KeyboardEvent) => {
    e.preventDefault();
    
    // Always log
    console.log("[kiklet][hotkey][ui] keydown", {
      key: e.key,
      code: e.code,
      meta: e.metaKey,
      ctrl: e.ctrlKey,
      alt: e.altKey,
      shift: e.shiftKey,
      repeat: e.repeat,
    });
    
    // Escape: cancel listening
    if (e.key === "Escape") {
      console.log("[kiklet] hotkey listening cancelled (Escape)");
      hotkeyCaptureActive = false;
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      setHotkeyMsg(null);
      void refreshHotkey();
      return;
    }
    
    // Backspace/Delete: clear hotkey
    if (e.key === "Backspace" || e.key === "Delete") {
      console.log("[kiklet] hotkey clear requested");
      hotkeyCaptureActive = false;
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      setHotkeyMsg(null);
      void refreshHotkey();
      return;
    }

    // Check if this is a modifier key
    if (isModifierKey(e)) {
      // Track pending modifier for modifier-only detection
      const modOnly = formatModifierOnly(e);
      if (modOnly) {
        pendingModifier = modOnly;
        hadNonModifier = false;
        els.hotkeyValue.textContent = `${modOnly}…`;
      }
      return; // Keep listening
    }

    // Non-modifier key: this is a combo
    hadNonModifier = true;
    pendingModifier = null;

    const isMac = navigator.platform.toLowerCase().includes("mac");
    const mods = {
      cmd: isMac && e.metaKey,
      ctrl: e.ctrlKey,
      alt: e.altKey,
      shift: e.shiftKey,
    };
    const code = normalizeCode(e.code);
    
    // Build accelerator string
    const parts: string[] = [];
    if (isMac) {
      if (e.metaKey) parts.push("Cmd");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.ctrlKey) parts.push("Ctrl");
    } else {
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.metaKey) parts.push("Win");
    }
    const candidate = parts.length > 0 ? `${parts.join("+")}+${code}` : code;

    console.log("[kiklet][hotkey][ui] candidate", { candidate, kind: "combo", code, mods });
    hotkeyCaptureActive = false;
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);

    void (async () => {
      setHotkeyMsg("Saving…");
      els.fieldHotkey.disabled = true;
      els.hotkeyReset.disabled = true;
      try {
        const saved = (await invoke("set_hotkey", {
          accelerator: candidate,
          kind: "combo",
          code: code,
          mods: mods,
        })) as string;
        els.hotkeyValue.textContent = saved;
        setHotkeyMsg("Saved");
        setTimeout(() => setHotkeyMsg(null), 450);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setHotkeyMsg(`Error: ${msg}`);
        console.error(`[kiklet][hotkey][ui] set_hotkey failed err=${msg}`);
        void refreshHotkey();
      } finally {
        els.fieldHotkey.disabled = false;
        els.hotkeyReset.disabled = false;
      }
    })();
  };

  const onKeyUp = (e: KeyboardEvent) => {
    // Modifier-only not supported - ignore
    if (pendingModifier && isModifierKey(e) && !hadNonModifier) {
      // Don't commit modifier-only, just cancel
      console.log("[kiklet][hotkey][ui] modifier-only not supported, cancelling");
      hotkeyCaptureActive = false;
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      setHotkeyMsg("Modifier-only not supported");
      setTimeout(() => setHotkeyMsg(null), 2000);
      void refreshHotkey();
    }
  };

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
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
  // Global JS error visibility (no silent failures)
  window.addEventListener("error", (ev) => {
    const msg = `[js-error] ${ev.message} @ ${ev.filename}:${ev.lineno}:${ev.colno}`;
    setJsError(msg);
    console.error(msg, (ev as any).error);
  });
  window.addEventListener("unhandledrejection", (ev) => {
    const msg = `[js-unhandledrejection] ${formatAnyError((ev as PromiseRejectionEvent).reason)}`;
    setJsError(msg);
    console.error(msg, (ev as PromiseRejectionEvent).reason);
  });

  loadUiPrefs();

  els.toggleAutostart.addEventListener("change", onAutostartToggle);
  els.toggleAutoInsert.addEventListener("change", onAutoInsertToggle);
  
  // Translate input handlers
  els.inputTranslate.addEventListener("focus", () => {
    if (els.inputTranslate.value === "Не переводить") {
      els.inputTranslate.value = "";
    }
  });
  
  els.inputTranslate.addEventListener("input", () => {
    const value = els.inputTranslate.value;
    if (value.trim() === "") {
      els.translateAutocomplete.style.display = "none";
      els.btnTranslateClear.style.display = "none";
    } else {
      els.btnTranslateClear.style.display = "block";
      showAutocomplete(value);
    }
  });
  
  els.inputTranslate.addEventListener("blur", () => {
    // Delay to allow click on autocomplete item
    setTimeout(() => {
      els.translateAutocomplete.style.display = "none";
      const value = els.inputTranslate.value.trim();
      if (value === "" || value === "Не переводить") {
        els.inputTranslate.value = "Не переводить";
        els.btnTranslateClear.style.display = "none";
        saveTranslateTarget();
      } else {
        saveTranslateTarget();
      }
    }, 200);
  });
  
  els.inputTranslate.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      els.inputTranslate.blur();
    } else if (e.key === "Escape") {
      els.translateAutocomplete.style.display = "none";
    }
  });
  
  els.btnTranslateClear.addEventListener("click", () => {
    els.inputTranslate.value = "Не переводить";
    els.btnTranslateClear.style.display = "none";
    els.translateAutocomplete.style.display = "none";
    saveTranslateTarget();
  });
  
  els.selectTranslateModel.addEventListener("change", async () => {
    const model = els.selectTranslateModel.value;
    try {
      await invoke("set_translate_model", { model });
      await refreshSettings(false);
    } catch (e) {
      console.error("[kiklet] set_translate_model failed:", e);
    }
  });
  
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
  
  // PTT threshold input handler (on blur)
  els.inputPttThreshold.addEventListener("blur", async () => {
    const rawValue = els.inputPttThreshold.value.trim();
    let value = 0;
    if (rawValue && !isNaN(Number(rawValue))) {
      value = Math.max(0, Math.floor(Number(rawValue)));
    }
    
    els.inputPttThreshold.value = String(value);
    
    try {
      const updated = (await invoke("set_ptt_threshold_ms", {
        thresholdMs: value,
      })) as SettingsDto;
      
      pttThresholdMs = updated.pttThresholdMs;
      
      // Apply runtime behavior
      if (value === 0) {
        console.log("[kiklet][ptt][ui] threshold=0 -> stop");
        try {
          await invoke("ptt_stop");
        } catch (e) {
          console.error("[kiklet][ptt][ui] ptt_stop error:", e);
        }
      } else {
        console.log(`[kiklet][ptt][ui] threshold=${value} -> start`);
        if (!updated.hotkeyCode || !updated.hotkeyMods) {
          console.log("[kiklet][ptt][ui] no hotkey -> skip start");
        } else {
          // Stop first to ensure idempotency
          try {
            await invoke("ptt_stop");
          } catch (e) {
            // Ignore errors on stop
          }
          try {
            const started = await invoke("ptt_start", {
              code: updated.hotkeyCode,
              mods: updated.hotkeyMods,
              accelerator: updated.hotkeyAccelerator || "",
            }) as boolean;
            if (started) {
              console.log("[kiklet][ptt][ui] start ok");
            }
          } catch (e) {
            console.error("[kiklet][ptt][ui] ptt_start error:", e);
          }
        }
      }
      
      setPttThresholdMsg("Saved");
      setTimeout(() => setPttThresholdMsg(null), 1000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("[kiklet][ptt][ui] set_ptt_threshold_ms failed:", e);
      setPttThresholdMsg(`Error: ${msg}`);
      setTimeout(() => setPttThresholdMsg(null), 3000);
      // Revert to previous value
      await refreshSettings(false);
    }
  });
  
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

  // Clear history button
  els.btnClearHistory.addEventListener("click", async () => {
    const originalText = els.btnClearHistory.textContent;
    try {
      setClearStatus("clear: clicked");

      // Visible proof that this handler runs (even without DevTools)
      els.btnClearHistory.textContent = "CLICKED";
      window.setTimeout(() => {
        if (els.btnClearHistory.textContent === "CLICKED") {
          els.btnClearHistory.textContent = originalText;
        }
      }, 400);

      setClearStatus("clear: before-confirm");
      console.log("[kiklet][ui] clear_all_recordings click");
      const confirmed = await confirm("Delete all recordings? This cannot be undone.", {
        title: "Clear history",
        kind: "warning",
      });

      setClearStatus(`clear: confirmed=${confirmed}`);
      if (!confirmed) return;

      els.btnClearHistory.disabled = true;
      els.btnClearHistory.textContent = "Clearing...";

      setClearStatus("clear: ping...");
      const pong = (await invoke("debug_ping")) as string;
      setClearStatus(`clear: ping ok (${pong})`);

      setClearStatus("clear: invoking clear_all_recordings");
      const result = (await invoke("clear_all_recordings")) as {
        deletedCount: number;
        recordingsDir: string;
        recordingsJson: string;
        filesOnDiskBefore: number;
        filesOnDiskAfter: number;
        indexCountBefore: number;
        indexCountAfter: number;
        failedDeletes: Array<{ filename: string; error: string }>;
      };

      setClearStatus(
        `clear: ok deletedCount=${result.deletedCount} failed=${result.failedDeletes?.length || 0} ` +
          `wavBefore=${result.filesOnDiskBefore} wavAfter=${result.filesOnDiskAfter} ` +
          `indexBefore=${result.indexCountBefore} indexAfter=${result.indexCountAfter}`,
      );

      if (result.failedDeletes && result.failedDeletes.length > 0) {
        showToast(`Error: failedDeletes=${result.failedDeletes.length}`);
      } else {
        showToast(`Cleared ${result.deletedCount} recordings`);
      }

      await refreshHistory();
    } catch (e) {
      const msg = formatAnyError(e);
      setClearStatus(`clear: ERROR ${msg}`);
      setJsError(`[clear handler] ${msg}`);
      showToast(`Error: ${msg}`);
      console.error("[kiklet][ui] clear_all_recordings handler failed", e);
    } finally {
      els.btnClearHistory.disabled = false;
      els.btnClearHistory.textContent = originalText;
    }
  });

  // Debug storage button: proves which recordings.json is read from disk
  els.btnDebugStorage.addEventListener("click", async () => {
    const originalText = els.btnDebugStorage.textContent;
    els.btnDebugStorage.textContent = "CLICKED";
    window.setTimeout(() => {
      if (els.btnDebugStorage.textContent === "CLICKED") {
        els.btnDebugStorage.textContent = originalText;
      }
    }, 400);

    els.btnDebugStorage.disabled = true;
    els.btnDebugStorage.textContent = "Loading…";
    els.debugStorageOut.hidden = true;
    els.debugStorageOut.textContent = "";

    try {
      const result = (await invoke("debug_dump_storage_paths")) as any;
      els.debugStorageOut.textContent = JSON.stringify(result, null, 2);
      els.debugStorageOut.hidden = false;
      showToast("Debug storage loaded");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      els.debugStorageOut.textContent = `Error: ${msg}`;
      els.debugStorageOut.hidden = false;
      showToast(`Error: ${msg}`);
      console.error("[kiklet][ui] debug_dump_storage_paths failed", e);
    } finally {
      els.btnDebugStorage.disabled = false;
      els.btnDebugStorage.textContent = originalText;
    }
  });

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

  // PTT: handle down/up events
  await listen("hotkey:down", async () => {
    console.log("[kiklet][ptt][ui] down");
    isHotkeyHeld = true;
    
    if (!isRecording) {
      // Not recording: set timer for PTT threshold
      pttArmed = true;
      pttActive = false;
      holdMode = false;
      
      // Clear any existing timer
      if (pttTimer !== null) {
        window.clearTimeout(pttTimer);
      }
      
      // After threshold, if still held, start recording and activate PTT mode
      pttTimer = window.setTimeout(async () => {
        if (isHotkeyHeld && !isRecording && pttArmed) {
          try {
            await start();
            pttActive = true;
            holdMode = true;
            console.log("[kiklet][ptt][ui] threshold->start holdMode=true");
          } catch (e) {
            console.error("[kiklet][ptt][ui] start on threshold failed:", e);
            pttArmed = false;
            pttActive = false;
            holdMode = false;
          }
        }
      }, pttThresholdMs);
      return;
    }
    
    // Recording already active
    if (pttActive && holdMode) {
      // In hold mode, ignore additional downs (wait for up)
      console.log("[kiklet][ptt][ui] down ignored (holdMode active)");
      return;
    } else if (isRecording && !holdMode) {
      // Toggle-stop: second tap stops (tap-to-toggle)
      try {
        await stop();
        console.log("[kiklet][ptt][ui] tap -> toggle stop");
        isHotkeyHeld = false;
        pttArmed = false;
        pttActive = false;
        holdMode = false;
        if (pttTimer !== null) {
          window.clearTimeout(pttTimer);
          pttTimer = null;
        }
      } catch (e) {
        console.error("[kiklet][ptt][ui] stop failed:", e);
        // Reset state anyway
        isHotkeyHeld = false;
        pttArmed = false;
        pttActive = false;
        holdMode = false;
        if (pttTimer !== null) {
          window.clearTimeout(pttTimer);
          pttTimer = null;
        }
      }
    }
  });

  await listen("hotkey:up", async () => {
    console.log("[kiklet][ptt][ui] up");
    isHotkeyHeld = false;
    
    // Clear timer
    if (pttTimer !== null) {
      window.clearTimeout(pttTimer);
      pttTimer = null;
    }
    
    // GUARANTEE: If holdMode is true, ALWAYS stop on release
    // Also stop if threshold became 0 during recording
    if ((holdMode && isRecording) || (pttThresholdMs === 0 && isRecording && pttActive)) {
      try {
        await stop();
        console.log("[kiklet][ptt][ui] up holdMode=true -> stop");
      } catch (e) {
        console.error("[kiklet][ptt][ui] stop on release failed:", e);
      }
      pttArmed = false;
      pttActive = false;
      holdMode = false;
      return;
    }
    
    // If up came before threshold (<300ms) and we're not recording, this is a tap
    if (!isRecording && pttArmed && !pttActive && !holdMode) {
      // Quick tap: start recording (toggle)
      try {
        await start();
        console.log("[kiklet][ptt][ui] tap -> toggle");
      } catch (e) {
        console.error("[kiklet][ptt][ui] start on tap failed:", e);
      }
    }
    
    // Reset PTT state
    pttArmed = false;
    pttActive = false;
    holdMode = false;
  });

  // Fallback: hotkey toggle (via tauri-plugin-global-shortcut, if PTT not available)
  await listen("hotkey:toggle-record", async () => {
    // Single-source-of-truth: ignore toggle-record if PTT is enabled (threshold_ms > 0)
    const s = (await invoke("get_settings")) as SettingsDto;
    const pttEnabled = (s.pttThresholdMs ?? 0) > 0;
    if (pttEnabled) {
      console.log("[kiklet][ptt][ui] ignore toggle-record (PTT enabled)");
      return;
    }
    
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
