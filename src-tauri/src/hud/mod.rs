use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

pub const HUD_WINDOW_LABEL: &str = "hud";

#[cfg(target_os = "macos")]
pub mod macos;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static GEN: AtomicU64 = AtomicU64::new(0);

// HUD window size (will be set in setup)
const HUD_WIDTH: f64 = 360.0;
const HUD_HEIGHT: f64 = 140.0;
// Offset from cursor: bottom-right
const OFFSET_X: f64 = 14.0;
const OFFSET_Y: f64 = 18.0;

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn activate(app: &AppHandle) -> Result<(), String> {
    ACTIVE.store(true, Ordering::Relaxed);
    let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;

    let w = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "hud window not found".to_string())?;

    let _ = w.show();

    // Note: In Tauri v2, getting NSWindow pointer directly is complex.
    // We use Tauri API with proper coordinate conversion.
    // The key is correct clamp in bottom-left origin before conversion.

    // Start cursor follow thread (macOS only; other OS no-op for now)
    let app_clone = app.clone();
    let mut debug_log_counter = 0u32;
    std::thread::spawn(move || {
        loop {
            if !ACTIVE.load(Ordering::Relaxed) {
                break;
            }
            if GEN.load(Ordering::Relaxed) != gen {
                break;
            }

            #[cfg(target_os = "macos")]
            {
                if let Some((mx_pts, my_pts, screen_id, visible_frame, scale)) = crate::hud::macos::cursor_pos_and_screen() {
                    // Get window size in physical pixels and convert to points
                    let (w_pts, h_pts) = if let Some(w) = app_clone.get_webview_window(HUD_WINDOW_LABEL) {
                        // Get outer_size - in Tauri v2 this returns PhysicalSize<u32> (physical pixels)
                        if let Ok(phys_size) = w.outer_size() {
                            // Convert physical pixels to points via scale factor
                            let w_pts = phys_size.width as f64 / scale;
                            let h_pts = phys_size.height as f64 / scale;
                            (w_pts, h_pts)
                        } else {
                            // Fallback to constants (assumed to be in points)
                            (HUD_WIDTH, HUD_HEIGHT)
                        }
                    } else {
                        (HUD_WIDTH, HUD_HEIGHT)
                    };

                    // Calculate top-left position in POINTS (bottom-left origin)
                    let (tx_pts, ty_pts) = crate::hud::macos::calc_hud_top_left(mx_pts, my_pts, w_pts, h_pts);
                    
                    // Clamp to screen visibleFrame in POINTS
                    let (clamped_x_pts, clamped_y_pts) = crate::hud::macos::clamp_to_screen(tx_pts, ty_pts, w_pts, h_pts, visible_frame);

                    // Convert from bottom-left to top-left for Tauri LogicalPosition
                    // Get virtual max Y across all screens (in points)
                    let virtual_max_y_pts = crate::hud::macos::get_virtual_max_y();
                    // ty_pts is top-left point in bottom-left origin, convert to top-left origin
                    let tauri_y_pts = virtual_max_y_pts - clamped_y_pts;

                    // Position window via Tauri API using LogicalPosition (POINTS)
                    if let Some(w) = app_clone.get_webview_window(HUD_WINDOW_LABEL) {
                        let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition {
                            x: clamped_x_pts,
                            y: tauri_y_pts,
                        }));
                    }

                    // Debug log (once per 400ms = ~25 times per 10s)
                    debug_log_counter += 1;
                    if debug_log_counter % 25 == 0 {
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "[kiklet][hud] scale={:.2} mouse_pts=({:.1},{:.1}) win_pts=({:.1},{:.1}) vf_pts=({:.1},{:.1},{:.1},{:.1}) topLeft_pts=({:.1},{:.1}) tauri_pos_pts=({:.1},{:.1}) screen={}",
                            scale,
                            mx_pts, my_pts,
                            w_pts, h_pts,
                            visible_frame.origin.x, visible_frame.origin.y,
                            visible_frame.origin.x + visible_frame.size.width,
                            visible_frame.origin.y + visible_frame.size.height,
                            clamped_x_pts, clamped_y_pts,
                            clamped_x_pts, tauri_y_pts,
                            screen_id
                        );
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(16)); // ~60Hz
        }
    });

    Ok(())
}

pub fn deactivate(app: &AppHandle) {
    ACTIVE.store(false, Ordering::Relaxed);
    GEN.fetch_add(1, Ordering::Relaxed);
    if let Some(w) = app.get_webview_window(HUD_WINDOW_LABEL) {
        let _ = w.hide();
    }
}

pub fn emit_status(app: &AppHandle, phase: &str, text: &str) {
    if !is_active() {
        return;
    }
    if let Some(w) = app.get_webview_window(HUD_WINDOW_LABEL) {
        let _ = w.emit(
            "hud_status",
            serde_json::json!({
                "phase": phase,
                "text": text
            }),
        );
    }
}

pub fn emit_level(app: &AppHandle, level: f32) {
    if !is_active() {
        return;
    }
    if let Some(w) = app.get_webview_window(HUD_WINDOW_LABEL) {
        let _ = w.emit("hud_level", serde_json::json!({ "level": level }));
    }
}


