#[cfg(target_os = "macos")]
use objc::runtime::{Class, Object};
#[cfg(target_os = "macos")]
use objc::{msg_send, sel, sel_impl};

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct NSSize {
    pub width: f64,
    pub height: f64,
}

/// Returns cursor position in screen coordinates (bottom-left origin, macOS standard, POINTS).
/// Also returns the screen ID, visibleFrame, and backingScaleFactor for the screen containing cursor.
#[cfg(target_os = "macos")]
pub fn cursor_pos_and_screen() -> Option<(f64, f64, usize, NSRect, f64)> {
    unsafe {
        let nsevent_class = Class::get("NSEvent")?;
        // NSEvent.mouseLocation() returns coordinates in screen space with bottom-left origin (POINTS)
        let loc: NSPoint = msg_send![nsevent_class, mouseLocation];

        // Find which screen contains the cursor
        let nsscreen_class = Class::get("NSScreen")?;
        let screens: *const Object = msg_send![nsscreen_class, screens];
        let screens_nsarray: &Object = &*(screens as *const Object);
        let count: usize = msg_send![screens_nsarray, count];

        for i in 0..count {
            let screen: *const Object = msg_send![screens_nsarray, objectAtIndex: i];
            let frame: NSRect = msg_send![screen, frame];
            // NSRect is Copy, access fields directly
            if loc.x >= frame.origin.x
                && loc.x < frame.origin.x + frame.size.width
                && loc.y >= frame.origin.y
                && loc.y < frame.origin.y + frame.size.height
            {
                // Get visibleFrame (accounts for menu bar, dock, etc.) - in POINTS
                let visible_frame: NSRect = msg_send![screen, visibleFrame];
                // Get backingScaleFactor (e.g., 2.0 for Retina)
                let scale: f64 = msg_send![screen, backingScaleFactor];
                return Some((loc.x, loc.y, i, visible_frame, scale));
            }
        }

        // Fallback: use first screen
        let first_screen: *const Object = msg_send![screens_nsarray, objectAtIndex: 0];
        let visible_frame: NSRect = msg_send![first_screen, visibleFrame];
        let scale: f64 = msg_send![first_screen, backingScaleFactor];
        Some((loc.x, loc.y, 0, visible_frame, scale))
    }
}

/// Get virtual maximum Y coordinate across all screens (in POINTS, bottom-left origin).
/// This is used for converting bottom-left coordinates to top-left.
#[cfg(target_os = "macos")]
pub fn get_virtual_max_y() -> f64 {
    unsafe {
        let nsscreen_class = match Class::get("NSScreen") {
            Some(c) => c,
            None => return 0.0,
        };
        let screens: *const Object = msg_send![nsscreen_class, screens];
        let screens_nsarray: &Object = &*(screens as *const Object);
        let count: usize = msg_send![screens_nsarray, count];

        let mut max_y = 0.0;
        for i in 0..count {
            let screen: *const Object = msg_send![screens_nsarray, objectAtIndex: i];
            let frame: NSRect = msg_send![screen, frame];
            let screen_top = frame.origin.y + frame.size.height;
            if screen_top > max_y {
                max_y = screen_top;
            }
        }
        max_y
    }
}

/// Calculate HUD top-left position from cursor position (in bottom-left origin screen space).
/// Returns (x, y) for NSWindow.setFrameTopLeftPoint.
#[cfg(target_os = "macos")]
pub fn calc_hud_top_left(mx: f64, my: f64, _w: f64, _h: f64) -> (f64, f64) {
    const OFFSET_X: f64 = 14.0;
    const OFFSET_Y: f64 = 18.0;
    // In bottom-left origin: "below" cursor means decreasing Y
    // top-left point of window for setFrameTopLeftPoint
    let x = mx + OFFSET_X;
    let y = my - OFFSET_Y;
    (x, y)
}

/// Clamp top-left position to screen visibleFrame bounds.
/// visibleFrame is in bottom-left origin.
/// For top-left point clamping:
/// - X: minX <= tx <= maxX - w
/// - Y: minY + h <= ty <= maxY (top-left Y must account for window height)
#[cfg(target_os = "macos")]
pub fn clamp_to_screen(tx: f64, ty: f64, w: f64, h: f64, visible_frame: NSRect) -> (f64, f64) {
    // visibleFrame is in bottom-left origin
    let min_x = visible_frame.origin.x;
    let max_x = visible_frame.origin.x + visible_frame.size.width;
    let min_y = visible_frame.origin.y;
    let max_y = visible_frame.origin.y + visible_frame.size.height;
    
    // Clamp top-left point:
    // X: window must fit within [minX, maxX]
    let clamped_x = tx.max(min_x).min(max_x - w);
    // Y: top-left Y must be at least (minY + h) and at most maxY
    let clamped_y = ty.max(min_y + h).min(max_y);

    (clamped_x, clamped_y)
}

/// Set NSWindow position using setFrameTopLeftPoint (native macOS API).
/// window_ptr: NSWindow pointer.
/// x, y: top-left coordinates in screen space (bottom-left origin).
#[cfg(target_os = "macos")]
pub fn set_window_position_native(window_ptr: *mut Object, x: f64, y: f64) -> bool {
    unsafe {
        let point = NSPoint { x, y };
        let _: () = msg_send![window_ptr, setFrameTopLeftPoint: point];
        true
    }
}

/// Get NSWindow frame size (in points).
#[cfg(target_os = "macos")]
pub fn get_window_size(window_ptr: *mut Object) -> Option<(f64, f64)> {
    unsafe {
        let frame: NSRect = msg_send![window_ptr, frame];
        Some((frame.size.width, frame.size.height))
    }
}


