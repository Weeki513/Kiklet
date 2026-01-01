use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use super::{DeliveryMode, DeliveryResult};

// Accessibility API types
type AXUIElementRef = *mut std::ffi::c_void;
type AXValueRef = *mut std::ffi::c_void;
type CFStringRef = *const std::ffi::c_void;
type CFTypeRef = *const std::ffi::c_void;
type CFBooleanRef = *const std::ffi::c_void;
type CFTypeID = u64;
type CFIndex = isize;

#[repr(C)]
struct CFRange {
    location: CFIndex,
    length: CFIndex,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32; // kAXErrorSuccess = 0
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32; // kAXErrorSuccess = 0
    fn AXValueCreate(the_type: u32, value_ptr: *const std::ffi::c_void) -> AXValueRef;
    fn AXValueGetType(value: AXValueRef) -> u32;
    fn AXValueGetValue(value: AXValueRef, the_type: u32, value_ptr: *mut std::ffi::c_void) -> bool;
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFBooleanGetValue(boolean: CFBooleanRef) -> bool;
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut i8,
        buffer_size: usize,
        encoding: u32,
    ) -> bool;
}

// AXValue types
const K_AX_VALUE_CF_RANGE_TYPE: u32 = 200; // kAXValueCFRangeType

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFBooleanRef;
    static kCFBooleanFalse: CFBooleanRef;
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        c_str: *const i8,
        encoding: u32,
    ) -> CFStringRef;
    fn CFBooleanGetTypeID() -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
}

// Accessibility attribute constants
const K_AX_FOCUSED_UI_ELEMENT: &[u8] = b"AXFocusedUIElement\0";
const K_AX_EDITABLE_ATTRIBUTE: &[u8] = b"AXEditableText\0";
const K_AX_ROLE_ATTRIBUTE: &[u8] = b"AXRole\0";
const K_AX_ENABLED_ATTRIBUTE: &[u8] = b"AXEnabled\0";
const K_AX_VALUE_ATTRIBUTE: &[u8] = b"AXValue\0";
const K_AX_SELECTED_TEXT_RANGE_ATTRIBUTE: &[u8] = b"AXSelectedTextRange\0";

// Role constants
const K_AX_TEXT_FIELD_ROLE: &[u8] = b"AXTextField\0";
const K_AX_TEXT_AREA_ROLE: &[u8] = b"AXTextArea\0";
const K_AX_COMBO_BOX_ROLE: &[u8] = b"AXComboBox\0";
const K_AX_SEARCH_FIELD_ROLE: &[u8] = b"AXSearchField\0";

fn create_cf_string(s: &[u8]) -> CFStringRef {
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            s.as_ptr() as *const i8,
            0x08000100, // kCFStringEncodingUTF8
        )
    }
}

fn cf_string_to_string(cf_str: CFStringRef) -> Option<String> {
    unsafe {
        let mut buffer = vec![0i8; 256];
        if CFStringGetCString(cf_str, buffer.as_mut_ptr(), buffer.len(), 0x08000100) {
            // kCFStringEncodingUTF8
            let c_str = std::ffi::CStr::from_ptr(buffer.as_ptr());
            c_str.to_str().ok().map(|s| s.to_string())
        } else {
            None
        }
    }
}

fn get_cf_string_value(cf_str: CFStringRef) -> String {
    // Try to get full string, may need larger buffer
    unsafe {
        // First try with 256 bytes
        let mut buffer = vec![0i8; 256];
        if CFStringGetCString(cf_str, buffer.as_mut_ptr(), buffer.len(), 0x08000100) {
            let c_str = std::ffi::CStr::from_ptr(buffer.as_ptr());
            if let Ok(s) = c_str.to_str() {
                return s.to_string();
            }
        }
        // Fallback: try with larger buffer
        let mut buffer = vec![0i8; 4096];
        if CFStringGetCString(cf_str, buffer.as_mut_ptr(), buffer.len(), 0x08000100) {
            let c_str = std::ffi::CStr::from_ptr(buffer.as_ptr());
            if let Ok(s) = c_str.to_str() {
                return s.to_string();
            }
        }
        String::new()
    }
}

fn has_focused_editable_element() -> Result<bool, String> {
    // Check Accessibility permission first
    if !crate::perm::macos::ax_is_trusted() {
        return Ok(false);
    }

    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return Err("failed to create system-wide AX element".to_string());
        }

        // Get focused element
        let mut focused_element: CFTypeRef = std::ptr::null_mut();
        let focused_attr = create_cf_string(K_AX_FOCUSED_UI_ELEMENT);
        let err = AXUIElementCopyAttributeValue(system_wide, focused_attr, &mut focused_element);
        CFRelease(focused_attr as *const std::ffi::c_void);

        if err != 0 {
            // kAXErrorSuccess = 0, any other value is an error
            CFRelease(system_wide as *const std::ffi::c_void);
            return Ok(false); // No focused element or error - not editable
        }

        if focused_element.is_null() {
            CFRelease(system_wide as *const std::ffi::c_void);
            return Ok(false);
        }

        // Check if element is enabled
        let enabled_attr = create_cf_string(K_AX_ENABLED_ATTRIBUTE);
        let mut enabled_value: CFTypeRef = std::ptr::null_mut();
        let enabled_err = AXUIElementCopyAttributeValue(
            focused_element as AXUIElementRef,
            enabled_attr,
            &mut enabled_value,
        );
        CFRelease(enabled_attr as *const std::ffi::c_void);
        if enabled_err == 0 && !enabled_value.is_null() {
            let type_id = CFGetTypeID(enabled_value);
            if type_id == CFBooleanGetTypeID() {
                let is_enabled = CFBooleanGetValue(enabled_value as CFBooleanRef);
                if !is_enabled {
                    CFRelease(enabled_value as *const std::ffi::c_void);
                    CFRelease(focused_element as *const std::ffi::c_void);
                    CFRelease(system_wide as *const std::ffi::c_void);
                    return Ok(false);
                }
            }
            CFRelease(enabled_value as *const std::ffi::c_void);
        }

        // Check if element is editable
        let editable_attr = create_cf_string(K_AX_EDITABLE_ATTRIBUTE);
        let mut editable_value: CFTypeRef = std::ptr::null_mut();
        let editable_err = AXUIElementCopyAttributeValue(
            focused_element as AXUIElementRef,
            editable_attr,
            &mut editable_value,
        );
        CFRelease(editable_attr as *const std::ffi::c_void);
        if editable_err == 0 && !editable_value.is_null() {
            let type_id = CFGetTypeID(editable_value);
            if type_id == CFBooleanGetTypeID() {
                let is_editable = CFBooleanGetValue(editable_value as CFBooleanRef);
                CFRelease(editable_value as *const std::ffi::c_void);
                CFRelease(focused_element as *const std::ffi::c_void);
                CFRelease(system_wide as *const std::ffi::c_void);
                return Ok(is_editable);
            }
            CFRelease(editable_value as *const std::ffi::c_void);
        }

        // Fallback: check role
        let role_attr = create_cf_string(K_AX_ROLE_ATTRIBUTE);
        let mut role_value: CFTypeRef = std::ptr::null_mut();
        let role_err = AXUIElementCopyAttributeValue(
            focused_element as AXUIElementRef,
            role_attr,
            &mut role_value,
        );
        CFRelease(role_attr as *const std::ffi::c_void);
        if role_err == 0 && !role_value.is_null() {
            let type_id = CFGetTypeID(role_value);
            if type_id == CFStringGetTypeID() {
                if let Some(role_str) = cf_string_to_string(role_value as CFStringRef) {
                    let role_bytes = role_str.as_bytes();
                    let is_text_field = role_bytes == b"AXTextField"
                        || role_bytes == b"AXTextArea"
                        || role_bytes == b"AXComboBox"
                        || role_bytes == b"AXSearchField";
                    CFRelease(role_value as *const std::ffi::c_void);
                    CFRelease(focused_element as *const std::ffi::c_void);
                    CFRelease(system_wide as *const std::ffi::c_void);
                    return Ok(is_text_field);
                }
            }
            CFRelease(role_value as *const std::ffi::c_void);
        }

        CFRelease(focused_element as *const std::ffi::c_void);
        CFRelease(system_wide as *const std::ffi::c_void);
        Ok(false)
    }
}

fn ax_insert_text_into_focused(text: &str) -> Result<(), String> {
    // Check Accessibility permission first
    if !crate::perm::macos::ax_is_trusted() {
        eprintln!("[kiklet][deliver][macos] ax_trusted=false");
        return Err("need_accessibility".to_string());
    }
    
    eprintln!("[kiklet][deliver][macos] ax_trusted=true");
    
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return Err("failed to create system-wide AX element".to_string());
        }
        
        // Get focused element
        let mut focused_element: CFTypeRef = std::ptr::null_mut();
        let focused_attr = create_cf_string(K_AX_FOCUSED_UI_ELEMENT);
        let err = AXUIElementCopyAttributeValue(system_wide, focused_attr, &mut focused_element);
        CFRelease(focused_attr as *const std::ffi::c_void);
        
        if err != 0 || focused_element.is_null() {
            CFRelease(system_wide as *const std::ffi::c_void);
            return Err("no_focused_element".to_string());
        }
        
        let focused_elem = focused_element as AXUIElementRef;
        
        // Check if element is editable
        let editable_attr = create_cf_string(K_AX_EDITABLE_ATTRIBUTE);
        let mut editable_value: CFTypeRef = std::ptr::null_mut();
        let editable_err = AXUIElementCopyAttributeValue(focused_elem, editable_attr, &mut editable_value);
        CFRelease(editable_attr as *const std::ffi::c_void);
        
        let is_editable = if editable_err == 0 && !editable_value.is_null() {
            let type_id = CFGetTypeID(editable_value);
            if type_id == CFBooleanGetTypeID() {
                let result = CFBooleanGetValue(editable_value as CFBooleanRef);
                CFRelease(editable_value as *const std::ffi::c_void);
                result
            } else {
                CFRelease(editable_value as *const std::ffi::c_void);
                false
            }
        } else {
            false
        };
        
        if !is_editable {
            // Fallback: check role
            let role_attr = create_cf_string(K_AX_ROLE_ATTRIBUTE);
            let mut role_value: CFTypeRef = std::ptr::null_mut();
            let role_err = AXUIElementCopyAttributeValue(focused_elem, role_attr, &mut role_value);
            CFRelease(role_attr as *const std::ffi::c_void);
            
            let is_text_field = if role_err == 0 && !role_value.is_null() {
                let type_id = CFGetTypeID(role_value);
                if type_id == CFStringGetTypeID() {
                    if let Some(role_str) = cf_string_to_string(role_value as CFStringRef) {
                        let role_bytes = role_str.as_bytes();
                        let result = role_bytes == b"AXTextField"
                            || role_bytes == b"AXTextArea"
                            || role_bytes == b"AXComboBox"
                            || role_bytes == b"AXSearchField";
                        CFRelease(role_value as *const std::ffi::c_void);
                        result
                    } else {
                        CFRelease(role_value as *const std::ffi::c_void);
                        false
                    }
                } else {
                    CFRelease(role_value as *const std::ffi::c_void);
                    false
                }
            } else {
                false
            };
            
            if !is_text_field {
                CFRelease(focused_element as *const std::ffi::c_void);
                CFRelease(system_wide as *const std::ffi::c_void);
                return Err("no_focused_editable".to_string());
            }
        }
        
        // Get current text value
        let value_attr = create_cf_string(K_AX_VALUE_ATTRIBUTE);
        let mut current_value: CFTypeRef = std::ptr::null_mut();
        let value_err = AXUIElementCopyAttributeValue(focused_elem, value_attr, &mut current_value);
        CFRelease(value_attr as *const std::ffi::c_void);
        
        let mut current_text = String::new();
        if value_err == 0 && !current_value.is_null() {
            let type_id = CFGetTypeID(current_value);
            if type_id == CFStringGetTypeID() {
                current_text = get_cf_string_value(current_value as CFStringRef);
            }
            CFRelease(current_value as *const std::ffi::c_void);
        }
        
        // Get selected text range (cursor position)
        let range_attr = create_cf_string(K_AX_SELECTED_TEXT_RANGE_ATTRIBUTE);
        let mut range_value: CFTypeRef = std::ptr::null_mut();
        let range_err = AXUIElementCopyAttributeValue(focused_elem, range_attr, &mut range_value);
        CFRelease(range_attr as *const std::ffi::c_void);
        
        let (insert_pos, replace_len) = if range_err == 0 && !range_value.is_null() {
            // range_value should be AXValueRef, not CFStringRef
            let ax_value = range_value as AXValueRef;
            let ax_type = AXValueGetType(ax_value);
            if ax_type == K_AX_VALUE_CF_RANGE_TYPE {
                let mut range = CFRange { location: 0, length: 0 };
                if AXValueGetValue(ax_value, K_AX_VALUE_CF_RANGE_TYPE, &mut range as *mut _ as *mut std::ffi::c_void) {
                    let pos = range.location as usize;
                    let len = range.length as usize;
                    CFRelease(range_value as *const std::ffi::c_void);
                    (pos, len)
                } else {
                    CFRelease(range_value as *const std::ffi::c_void);
                    (current_text.len(), 0) // Fallback: append at end
                }
            } else {
                CFRelease(range_value as *const std::ffi::c_void);
                (current_text.len(), 0) // Fallback: append at end
            }
        } else {
            (current_text.len(), 0) // Fallback: append at end
        };
        
        // Build new text: replace selected range with inserted text
        let new_text = if insert_pos <= current_text.len() {
            let before = &current_text[..insert_pos];
            let after_start = (insert_pos + replace_len).min(current_text.len());
            let after = &current_text[after_start..];
            format!("{}{}{}", before, text, after)
        } else {
            format!("{}{}", current_text, text)
        };
        
        // Set new value
        let new_value_cf = create_cf_string(new_text.as_bytes());
        let set_err = AXUIElementSetAttributeValue(focused_elem, create_cf_string(K_AX_VALUE_ATTRIBUTE), new_value_cf);
        CFRelease(new_value_cf as *const std::ffi::c_void);
        
        if set_err != 0 {
            CFRelease(focused_element as *const std::ffi::c_void);
            CFRelease(system_wide as *const std::ffi::c_void);
            return Err(format!("ax_set_value_failed err={}", set_err));
        }
        
        // Set cursor position after inserted text
        let new_cursor_pos = insert_pos + text.len();
        let new_range = CFRange {
            location: new_cursor_pos as CFIndex,
            length: 0,
        };
        let range_ax_value = AXValueCreate(K_AX_VALUE_CF_RANGE_TYPE, &new_range as *const _ as *const std::ffi::c_void);
        if !range_ax_value.is_null() {
            let range_attr = create_cf_string(K_AX_SELECTED_TEXT_RANGE_ATTRIBUTE);
            let _ = AXUIElementSetAttributeValue(focused_elem, range_attr, range_ax_value);
            CFRelease(range_attr as *const std::ffi::c_void);
            CFRelease(range_ax_value as *const std::ffi::c_void);
        }
        
        CFRelease(focused_element as *const std::ffi::c_void);
        CFRelease(system_wide as *const std::ffi::c_void);
        
        eprintln!("[kiklet][deliver][macos] ax_insert_ok");
        Ok(())
    }
}

fn run_capture(cmd: &str, args: &[&str]) -> Result<(i32, String, String), String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {cmd}: {e}"))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn run_with_stdin(cmd: &str, args: &[&str], input: &[u8]) -> Result<i32, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {cmd}: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input)
            .map_err(|e| format!("failed to write stdin for {cmd}: {e}"))?;
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for {cmd}: {e}"))?;
    if out.status.success() {
        Ok(out.status.code().unwrap_or(0))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

fn trunc200(s: &str) -> String {
    let mut out = s.replace('\n', " ").replace('\r', " ");
    if out.len() > 200 {
        out.truncate(200);
    }
    out
}

type CGEventRef = *mut std::ffi::c_void;
type CGEventSourceRef = *mut std::ffi::c_void;
type CGEventTapLocation = u32;
type CGKeyCode = u16;
type CGEventFlags = u64;

const KCG_HID_EVENT_TAP: CGEventTapLocation = 0;
const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;

// Common macOS virtual key codes
const KVK_ANSI_V: CGKeyCode = 0x09;
const KVK_COMMAND: CGKeyCode = 0x37;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: CGKeyCode,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventPost(tap: CGEventTapLocation, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
}

fn post_key(key: CGKeyCode, down: bool, flags: CGEventFlags) -> Result<(), String> {
    unsafe {
        let ev = CGEventCreateKeyboardEvent(std::ptr::null_mut(), key, down);
        if ev.is_null() {
            return Err("failed to create CGEvent".to_string());
        }
        if flags != 0 {
            CGEventSetFlags(ev, flags);
        }
        CGEventPost(KCG_HID_EVENT_TAP, ev);
        CFRelease(ev as *const std::ffi::c_void);
        Ok(())
    }
}

fn try_paste_cmd_v() -> Result<(), String> {
    // Cmd down
    post_key(KVK_COMMAND, true, 0)?;
    sleep(Duration::from_millis(15));
    // V down/up with command flag
    post_key(KVK_ANSI_V, true, KCG_EVENT_FLAG_MASK_COMMAND)?;
    sleep(Duration::from_millis(10));
    post_key(KVK_ANSI_V, false, KCG_EVENT_FLAG_MASK_COMMAND)?;
    sleep(Duration::from_millis(10));
    // Cmd up
    post_key(KVK_COMMAND, false, 0)?;
    Ok(())
}

pub fn deliver(text: &str, attempt_insert: bool) -> Result<DeliveryResult, String> {
    eprintln!("[kiklet][deliver][macos] start len={} attempt_insert={}", text.len(), attempt_insert);
    
    // Step 1: If autoinsert is disabled, always use clipboard
    if !attempt_insert {
        eprintln!("[kiklet][deliver][macos] attempt_insert=false -> clipboard_only");
        match run_with_stdin("pbcopy", &[], text.as_bytes()) {
            Ok(_) => {
                eprintln!("[kiklet][deliver][macos] clipboard_set len={}", text.len());
                return Ok(DeliveryResult {
                    mode: DeliveryMode::Copy,
                    ok: true,
                    detail: Some("clipboard_only_autoinsert_off".to_string()),
                });
            }
            Err(e) => {
                eprintln!("[kiklet][deliver][macos] pbcopy failed: {}", trunc200(&e));
                return Err(format!("clipboard_failed: {}", trunc200(&e)));
            }
        }
    }
    
    // Step 2: attempt_insert == true, try AX direct insert first (without touching clipboard)
    eprintln!("[kiklet][deliver][macos] attempt_insert=true -> try_ax_insert");
    match ax_insert_text_into_focused(text) {
        Ok(()) => {
            // Success: text inserted directly, clipboard NOT touched
            return Ok(DeliveryResult {
                mode: DeliveryMode::Insert,
                ok: true,
                detail: Some("ax_insert_ok".to_string()),
            });
        }
        Err(e) => {
            let err_str = trunc200(&e);
            eprintln!("[kiklet][deliver][macos] ax_insert_fail err={} -> clipboard_fallback", err_str);
            
            // Fallback: set clipboard
            match run_with_stdin("pbcopy", &[], text.as_bytes()) {
                Ok(_) => {
                    eprintln!("[kiklet][deliver][macos] clipboard_set len={}", text.len());
                }
                Err(clip_err) => {
                    eprintln!("[kiklet][deliver][macos] pbcopy failed: {}", trunc200(&clip_err));
                    return Err(format!("clipboard_failed: {}", trunc200(&clip_err)));
                }
            }
            
            // Return appropriate detail based on error
            let detail = if e == "need_accessibility" {
                "need_accessibility_clipboard"
            } else if e == "no_focused_editable" || e == "no_focused_element" {
                "no_focused_editable_clipboard"
            } else {
                &format!("ax_insert_failed_clipboard:{}", err_str)
            };
            
            return Ok(DeliveryResult {
                mode: DeliveryMode::Copy,
                ok: true,
                detail: Some(detail.to_string()),
            });
        }
    }
}


