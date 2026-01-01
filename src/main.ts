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

const els = {
  status: document.getElementById("status") as HTMLDivElement,
  btnStart: document.getElementById("btn-start") as HTMLButtonElement,
  btnStop: document.getElementById("btn-stop") as HTMLButtonElement,
  btnFolder: document.getElementById("btn-folder") as HTMLButtonElement,
  items: document.getElementById("items") as HTMLDivElement,
  count: document.getElementById("count") as HTMLDivElement,
  audio: document.getElementById("audio") as HTMLAudioElement,
};

let isRecording = false;

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
  els.status.textContent = isRecording ? "Recording…" : "Idle";
  els.btnStart.disabled = isRecording;
  els.btnStop.disabled = !isRecording;
}

function itemSub(r: RecordingItem): string {
  return `${fmtDuration(r.durationSec)} • ${fmtBytes(r.sizeBytes)}`;
}

function renderItems(items: RecordingItem[]) {
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

window.addEventListener("DOMContentLoaded", async () => {
  els.btnStart.addEventListener("click", start);
  els.btnStop.addEventListener("click", stop);
  els.btnFolder.addEventListener("click", openFolder);

  setRecordingState(false);
  await refresh();

  await listen<boolean>("recording_state", (event) => {
    setRecordingState(Boolean(event.payload));
  });
  await listen("recordings_updated", async () => {
    await refresh();
  });
});
