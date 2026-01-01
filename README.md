# Kiklet

Kiklet is a tiny, minimal-load desktop “listen” utility:

- Lives in the tray / menu bar
- Global hotkey toggles microphone recording
- Stores recordings locally as WAV (PCM)
- Shows a minimal history list + basic playback

No cloud, no transcription/LLM (yet). The Rust core is structured to extend cleanly later.

## Prerequisites

- **Rust**: stable toolchain (`rustup`, `cargo`)
- **Node.js**: LTS recommended
- **pnpm**: preferred (npm works too)

macOS must grant microphone permission to the app during first use.

## Setup

Using pnpm:

```bash
cd kiklet
pnpm install
```

Using npm:

```bash
cd kiklet
npm install
```

## Run (dev)

Using pnpm:

```bash
pnpm tauri dev
```

Using npm:

```bash
npm run tauri dev
```

## Build (release)

Using pnpm:

```bash
pnpm tauri build
```

Using npm:

```bash
npm run tauri build
```

## Hotkey

- **macOS default**: `Cmd+Shift+Space` toggles recording.
- Tray menu also provides Start/Stop.

Kiklet shows a small system notification when recording starts/stops.

## macOS microphone permission

If recording fails or you don’t see a permission prompt:

- System Settings → Privacy & Security → Microphone → enable **Kiklet**

## Storage location

Recordings are stored under the app data directory:

- **macOS**: `~/Library/Application Support/com.kiklet.app/recordings`
- The recordings index is: `~/Library/Application Support/com.kiklet.app/recordings.json`

Each file is named:

- `YYYY-MM-DD_HH-mm-ss.wav`

## Commands (Rust ↔ UI contract)

The app exposes these Tauri commands:

- `start_recording()`
- `stop_recording()`
- `list_recordings()`
- `reveal_in_finder(path)`
- `open_recordings_folder()`

## How to test API key Save (manual)

1) Run: `pnpm tauri dev`
2) On first launch the “OpenAI API key” modal appears.
3) Paste your key, click **Save** → modal shows “Saving…”, then “Saved.” and closes.
4) Quit and re-run `pnpm tauri dev` → modal should NOT appear (key persisted).
5) Click **Settings** → modal opens and shows the stored key (masked input).
