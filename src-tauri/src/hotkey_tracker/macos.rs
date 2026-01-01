use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use tauri::{AppHandle, Emitter};

use crate::modifier_hotkey;
use super::HotkeyConfig;

// FFI bindings for CoreGraphics
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: extern "C" fn(*const c_void, u32, *const c_void, *const c_void) -> *const c_void,
        user_info: *const c_void,
    ) -> *const c_void;
    fn CGEventTapEnable(tap: *const c_void, enable: bool);
    fn CGEventGetType(event: *const c_void) -> u32;
    fn CGEventGetFlags(event: *const c_void) -> u64;
    fn CGEventGetIntegerValueField(event: *const c_void, field: u64) -> i64;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *const c_void,
        order: i32,
    ) -> *const c_void;
    fn CFRunLoopGetCurrent() -> *const c_void;
    fn CFRunLoopAddSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: *const c_void);
    fn CFRelease(cf: *const c_void);
}

const kCGEventTapOptionDefault: u32 = 0;
const kCGHeadInsertEventTap: u32 = 0;
const kCGEventTapOptionListenOnly: u32 = 1;
const kCGEventKeyDown: u64 = 10;
const kCGEventKeyUp: u64 = 11;
const kCGEventFlagsChanged: u64 = 12;

const kCGEventFlagMaskCommand: u64 = 1 << 1;
const kCGEventFlagMaskAlternate: u64 = 1 << 3;
const kCGEventFlagMaskShift: u64 = 1 << 17;
const kCGEventFlagMaskControl: u64 = 1 << 18;

const kCGKeyboardEventKeycode: u64 = 9;

// Map code to macOS virtual keycode (simplified mapping)
fn code_to_vk(code: &str) -> Option<u16> {
    match code {
        "KeyA" => Some(0),
        "KeyS" => Some(1),
        "KeyD" => Some(2),
        "KeyF" => Some(3),
        "KeyH" => Some(4),
        "KeyG" => Some(5),
        "KeyZ" => Some(6),
        "KeyX" => Some(7),
        "KeyC" => Some(8),
        "KeyV" => Some(9),
        "KeyB" => Some(11),
        "KeyQ" => Some(12),
        "KeyW" => Some(13),
        "KeyE" => Some(14),
        "KeyR" => Some(15),
        "KeyY" => Some(16),
        "KeyT" => Some(17),
        "Key1" => Some(18),
        "Key2" => Some(19),
        "Key3" => Some(20),
        "Key4" => Some(21),
        "Key6" => Some(22),
        "Key5" => Some(23),
        "Equal" => Some(24),
        "Key9" => Some(25),
        "Key7" => Some(26),
        "Minus" => Some(27),
        "Key8" => Some(28),
        "Key0" => Some(29),
        "BracketRight" => Some(30),
        "KeyO" => Some(31),
        "KeyU" => Some(32),
        "BracketLeft" => Some(33),
        "KeyI" => Some(34),
        "KeyP" => Some(35),
        "Enter" => Some(36),
        "KeyL" => Some(37),
        "KeyJ" => Some(38),
        "Quote" => Some(39),
        "KeyK" => Some(40),
        "Semicolon" => Some(41),
        "Backslash" => Some(42),
        "Comma" => Some(43),
        "Slash" => Some(44),
        "KeyN" => Some(45),
        "KeyM" => Some(46),
        "Period" => Some(47),
        "Tab" => Some(48),
        "Space" => Some(49),
        "Backquote" => Some(50),
        "Backspace" => Some(51),
        "Escape" => Some(53),
        "F1" => Some(122),
        "F2" => Some(120),
        "F3" => Some(99),
        "F4" => Some(118),
        "F5" => Some(96),
        "F6" => Some(97),
        "F7" => Some(98),
        "F8" => Some(100),
        "F9" => Some(101),
        "F10" => Some(109),
        "F11" => Some(103),
        "F12" => Some(111),
        "ArrowUp" => Some(126),
        "ArrowDown" => Some(125),
        "ArrowLeft" => Some(123),
        "ArrowRight" => Some(124),
        "PageUp" => Some(116),
        "PageDown" => Some(121),
        "Home" => Some(115),
        "End" => Some(119),
        "Insert" => Some(114),
        "Delete" => Some(117),
        _ => {
            // Try to extract from Digit/Key prefix
            if code.starts_with("Digit") {
                let num = code.strip_prefix("Digit")?;
                num.parse::<u16>().ok().map(|n| n + 18) // Digits start at 18
            } else if code.starts_with("Key") {
                // Already handled above
                None
            } else {
                None
            }
        }
    }
}

struct TrackerState {
    config: HotkeyConfig,
    app: AppHandle,
    thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
    last_flags: Arc<Mutex<u64>>,
    key_pressed: Arc<Mutex<bool>>, // For combo: track if main key is pressed
    event_tap: usize,
    run_loop: usize,
    source: usize,
}

static STATE: OnceLock<Arc<Mutex<Option<TrackerState>>>> = OnceLock::new();

fn k_cf_run_loop_default_mode() -> *const c_void {
    std::ptr::null()
}

extern "C" fn event_tap_callback(
    _proxy: *const c_void,
    _type: u32,
    event: *const c_void,
    user_info: *const c_void,
) -> *const c_void {
    unsafe {
        if user_info.is_null() {
            return event;
        }
        
        let state_ptr = user_info as *const Arc<Mutex<Option<(HotkeyConfig, AppHandle, Arc<Mutex<bool>>, Arc<Mutex<u64>>, Arc<Mutex<bool>>)>>>;
        let state_arc = &*state_ptr;
        
        if let Ok(guard) = state_arc.lock() {
            if let Some((config, app, stop_flag, last_flags, key_pressed)) = guard.as_ref() {
                if let Ok(stop) = stop_flag.lock() {
                    if *stop {
                        return event;
                    }
                }
                
                let event_type = CGEventGetType(event);
                let flags = CGEventGetFlags(event);
                
                match config.kind.as_str() {
                    "modifier" => {
                        if event_type == kCGEventFlagsChanged as u32 {
                            if let Some(modifier) = &config.modifier {
                                let was_pressed = {
                                    let mut last = last_flags.lock().unwrap();
                                    let was = match modifier {
                                        modifier_hotkey::ModifierKey::Cmd => (*last & kCGEventFlagMaskCommand) != 0,
                                        modifier_hotkey::ModifierKey::Option => (*last & kCGEventFlagMaskAlternate) != 0,
                                        modifier_hotkey::ModifierKey::Shift => (*last & kCGEventFlagMaskShift) != 0,
                                        modifier_hotkey::ModifierKey::Ctrl => (*last & kCGEventFlagMaskControl) != 0,
                                        modifier_hotkey::ModifierKey::Win => false,
                                    };
                                    *last = flags;
                                    was
                                };
                                
                                let is_pressed = match modifier {
                                    modifier_hotkey::ModifierKey::Cmd => (flags & kCGEventFlagMaskCommand) != 0,
                                    modifier_hotkey::ModifierKey::Option => (flags & kCGEventFlagMaskAlternate) != 0,
                                    modifier_hotkey::ModifierKey::Shift => (flags & kCGEventFlagMaskShift) != 0,
                                    modifier_hotkey::ModifierKey::Ctrl => (flags & kCGEventFlagMaskControl) != 0,
                                    modifier_hotkey::ModifierKey::Win => false,
                                };
                                
                                if is_pressed && !was_pressed {
                                    eprintln!("[kiklet][hotkey] down kind=modifier");
                                    let _ = app.emit("hotkey:down", ());
                                } else if !is_pressed && was_pressed {
                                    eprintln!("[kiklet][hotkey] up kind=modifier");
                                    let _ = app.emit("hotkey:up", ());
                                }
                            }
                        }
                    }
                    "combo" => {
                        if let (Some(code), Some(mods)) = (&config.code, &config.mods) {
                            if event_type == kCGEventKeyDown as u32 || event_type == kCGEventKeyUp as u32 {
                                let vk = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
                                
                                // Check if this is our key
                                if let Some(target_vk) = code_to_vk(code) {
                                    if vk == target_vk {
                                        // Check modifiers match
                                        let mods_match = 
                                            (!mods.cmd || (flags & kCGEventFlagMaskCommand) != 0) &&
                                            (!mods.ctrl || (flags & kCGEventFlagMaskControl) != 0) &&
                                            (!mods.alt || (flags & kCGEventFlagMaskAlternate) != 0) &&
                                            (!mods.shift || (flags & kCGEventFlagMaskShift) != 0);
                                        
                                        if mods_match {
                                            if event_type == kCGEventKeyDown as u32 {
                                                let was_pressed = {
                                                    let mut pressed = key_pressed.lock().unwrap();
                                                    let was = *pressed;
                                                    *pressed = true;
                                                    was
                                                };
                                                
                                                if !was_pressed {
                                                    eprintln!("[kiklet][hotkey] down kind=combo code={}", code);
                                                    let _ = app.emit("hotkey:down", ());
                                                }
                                            } else if event_type == kCGEventKeyUp as u32 {
                                                let was_pressed = {
                                                    let mut pressed = key_pressed.lock().unwrap();
                                                    let was = *pressed;
                                                    *pressed = false;
                                                    was
                                                };
                                                
                                                if was_pressed {
                                                    eprintln!("[kiklet][hotkey] up kind=combo code={}", code);
                                                    let _ = app.emit("hotkey:up", ());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    event
}

pub fn start(app: &AppHandle, config: HotkeyConfig) -> Result<(), String> {
    let _ = stop(app);
    
    eprintln!("[kiklet][hotkey_tracker][mac] start kind={}", config.kind);
    
    unsafe {
        let events_of_interest = if config.kind == "modifier" {
            kCGEventFlagsChanged
        } else {
            kCGEventKeyDown | kCGEventKeyUp | kCGEventFlagsChanged
        };
        
        let event_tap = CGEventTapCreate(
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            kCGEventTapOptionListenOnly,
            events_of_interest,
            event_tap_callback,
            std::ptr::null(),
        );
        
        if event_tap.is_null() {
            return Err("failed to create event tap (need Accessibility permission?)".to_string());
        }
        
        CGEventTapEnable(event_tap, true);
        
        let source = CFMachPortCreateRunLoopSource(
            std::ptr::null(),
            event_tap,
            0,
        );
        
        if source.is_null() {
            CFRelease(event_tap);
            return Err("failed to create run loop source".to_string());
        }
        
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, k_cf_run_loop_default_mode());
        
        let stop_flag = Arc::new(Mutex::new(false));
        let last_flags = Arc::new(Mutex::new(0u64));
        let key_pressed = Arc::new(Mutex::new(false));
        
        let state_arc = Arc::new(Mutex::new(Some((
            config.clone(),
            app.clone(),
            stop_flag.clone(),
            last_flags.clone(),
            key_pressed.clone(),
        ))));
        let state_ptr = Arc::into_raw(state_arc.clone()) as *const c_void;
        
        CFRelease(event_tap);
        let event_tap2 = CGEventTapCreate(
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            kCGEventTapOptionListenOnly,
            events_of_interest,
            event_tap_callback,
            state_ptr,
        );
        
        if event_tap2.is_null() {
            CFRelease(source);
            return Err("failed to create event tap with user info".to_string());
        }
        
        CGEventTapEnable(event_tap2, true);
        
        let run_loop_thread = run_loop;
        let thread_handle = thread::spawn(move || {
            unsafe {
                CFRunLoopRun();
            }
        });
        
        let state = TrackerState {
            config,
            app: app.clone(),
            thread: Some(thread_handle),
            stop_flag,
            last_flags,
            key_pressed,
            event_tap: event_tap2 as usize,
            run_loop: run_loop_thread as usize,
            source: source as usize,
        };
        
        STATE.set(Arc::new(Mutex::new(Some(state)))).map_err(|_| "state already set".to_string())?;
        
        eprintln!("[kiklet][hotkey_tracker][mac] start ok");
        Ok(())
    }
}

pub fn stop(_app: &AppHandle) -> Result<(), String> {
    if let Some(state_arc) = STATE.get() {
        if let Ok(mut guard) = state_arc.lock() {
            if let Some(state) = guard.take() {
                eprintln!("[kiklet][hotkey_tracker][mac] stop");
                
                if let Ok(mut stop) = state.stop_flag.lock() {
                    *stop = true;
                }
                
                unsafe {
                    let rl = state.run_loop as *const c_void;
                    let src = state.source as *const c_void;
                    let tap = state.event_tap as *const c_void;
                    
                    CFRunLoopStop(rl);
                    if !src.is_null() {
                        CFRelease(src);
                    }
                    if !tap.is_null() {
                        CFRelease(tap);
                    }
                }
                
                if let Some(thread) = state.thread {
                    let _ = thread.join();
                }
                
                eprintln!("[kiklet][hotkey_tracker][mac] stop ok");
            }
        }
    }
    Ok(())
}

