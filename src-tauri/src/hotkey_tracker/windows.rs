use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use tauri::{AppHandle, Emitter};

use crate::modifier_hotkey;
use super::HotkeyConfig;

// FFI bindings for Windows API
#[link(name = "user32")]
extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        lpfn: extern "system" fn(i32, usize, isize) -> isize,
        h_mod: *const c_void,
        dw_thread_id: u32,
    ) -> isize;
    fn CallNextHookEx(h_hook: isize, n_code: i32, w_param: usize, l_param: isize) -> isize;
    fn UnhookWindowsHookEx(h_hook: isize) -> i32;
    fn GetMessageW(lp_msg: *mut MSG, h_wnd: *const c_void, w_msg_filter_min: u32, w_msg_filter_max: u32) -> i32;
    fn TranslateMessage(lp_msg: *const MSG) -> i32;
    fn DispatchMessageW(lp_msg: *const MSG) -> isize;
    fn PostThreadMessageW(id_thread: u32, msg: u32, w_param: usize, l_param: isize) -> i32;
    fn GetAsyncKeyState(v_key: i32) -> i16;
}

#[repr(C)]
struct MSG {
    hwnd: *const c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct KBDLLHOOKSTRUCT {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    dw_extra_info: usize,
}

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_QUIT: u32 = 0x0012;
const LLKHF_UP: u32 = 0x0080;

const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_MENU: u32 = 0x12; // Alt
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;

// Map code to Windows VK code
fn code_to_vk(code: &str) -> Option<u32> {
    match code {
        "KeyA" => Some(0x41),
        "KeyB" => Some(0x42),
        "KeyC" => Some(0x43),
        "KeyD" => Some(0x44),
        "KeyE" => Some(0x45),
        "KeyF" => Some(0x46),
        "KeyG" => Some(0x47),
        "KeyH" => Some(0x48),
        "KeyI" => Some(0x49),
        "KeyJ" => Some(0x4A),
        "KeyK" => Some(0x4B),
        "KeyL" => Some(0x4C),
        "KeyM" => Some(0x4D),
        "KeyN" => Some(0x4E),
        "KeyO" => Some(0x4F),
        "KeyP" => Some(0x50),
        "KeyQ" => Some(0x51),
        "KeyR" => Some(0x52),
        "KeyS" => Some(0x53),
        "KeyT" => Some(0x54),
        "KeyU" => Some(0x55),
        "KeyV" => Some(0x56),
        "KeyW" => Some(0x57),
        "KeyX" => Some(0x58),
        "KeyY" => Some(0x59),
        "KeyZ" => Some(0x5A),
        "Digit1" | "1" => Some(0x31),
        "Digit2" | "2" => Some(0x32),
        "Digit3" | "3" => Some(0x33),
        "Digit4" | "4" => Some(0x34),
        "Digit5" | "5" => Some(0x35),
        "Digit6" | "6" => Some(0x36),
        "Digit7" | "7" => Some(0x37),
        "Digit8" | "8" => Some(0x38),
        "Digit9" | "9" => Some(0x39),
        "Digit0" | "0" => Some(0x30),
        "Space" => Some(0x20),
        "Enter" => Some(0x0D),
        "Tab" => Some(0x09),
        "Escape" => Some(0x1B),
        "Backspace" => Some(0x08),
        "Delete" => Some(0x2E),
        "Insert" => Some(0x2D),
        "Home" => Some(0x24),
        "End" => Some(0x23),
        "PageUp" => Some(0x21),
        "PageDown" => Some(0x22),
        "ArrowUp" => Some(0x26),
        "ArrowDown" => Some(0x28),
        "ArrowLeft" => Some(0x25),
        "ArrowRight" => Some(0x27),
        "F1" => Some(0x70),
        "F2" => Some(0x71),
        "F3" => Some(0x72),
        "F4" => Some(0x73),
        "F5" => Some(0x74),
        "F6" => Some(0x75),
        "F7" => Some(0x76),
        "F8" => Some(0x77),
        "F9" => Some(0x78),
        "F10" => Some(0x79),
        "F11" => Some(0x7A),
        "F12" => Some(0x7B),
        _ => {
            if code.starts_with("Digit") {
                let num = code.strip_prefix("Digit")?;
                num.parse::<u32>().ok().map(|n| 0x30 + n)
            } else if code.starts_with("Key") {
                let letter = code.strip_prefix("Key")?;
                if letter.len() == 1 {
                    let ch = letter.chars().next()?;
                    Some(ch.to_ascii_uppercase() as u32)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

struct TrackerState {
    config: HotkeyConfig,
    app: AppHandle,
    hook: isize,
    thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
    pressed: Arc<Mutex<bool>>,
    key_pressed: Arc<Mutex<bool>>, // For combo: main key pressed
}

static STATE: OnceLock<Arc<Mutex<Option<TrackerState>>>> = OnceLock::new();
static HOOK_USER_DATA: OnceLock<Arc<Mutex<Option<(HotkeyConfig, AppHandle)>>>> = OnceLock::new();

extern "system" fn keyboard_hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
    if n_code >= 0 {
        unsafe {
            let hook_struct = *(l_param as *const KBDLLHOOKSTRUCT);
            let is_up = (hook_struct.flags & LLKHF_UP) != 0;
            let vk_code = hook_struct.vk_code;
            
            if let Some(state_arc) = HOOK_USER_DATA.get() {
                if let Ok(guard) = state_arc.lock() {
                    if let Some((config, app)) = guard.as_ref() {
                        match config.kind.as_str() {
                            "modifier" => {
                                if let Some(modifier) = &config.modifier {
                                    let is_our_modifier = match modifier {
                                        modifier_hotkey::ModifierKey::Cmd | modifier_hotkey::ModifierKey::Win => {
                                            vk_code == VK_LWIN || vk_code == VK_RWIN
                                        }
                                        modifier_hotkey::ModifierKey::Option => vk_code == VK_MENU,
                                        modifier_hotkey::ModifierKey::Shift => vk_code == VK_SHIFT,
                                        modifier_hotkey::ModifierKey::Ctrl => vk_code == VK_CONTROL,
                                    };
                                    
                                    if is_our_modifier {
                                        if let Some(state_arc) = STATE.get() {
                                            if let Ok(guard) = state_arc.lock() {
                                                if let Some(state) = guard.as_ref() {
                                                    if !is_up {
                                                        let was_pressed = {
                                                            let mut pressed = state.pressed.lock().unwrap();
                                                            let was = *pressed;
                                                            *pressed = true;
                                                            was
                                                        };
                                                        
                                                        if !was_pressed {
                                                            eprintln!("[kiklet][hotkey] down kind=modifier");
                                                            let _ = app.emit("hotkey:down", ());
                                                        }
                                                    } else {
                                                        let was_pressed = {
                                                            let mut pressed = state.pressed.lock().unwrap();
                                                            let was = *pressed;
                                                            *pressed = false;
                                                            was
                                                        };
                                                        
                                                        if was_pressed {
                                                            eprintln!("[kiklet][hotkey] up kind=modifier");
                                                            let _ = app.emit("hotkey:up", ());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "combo" => {
                                if let (Some(code), Some(mods)) = (&config.code, &config.mods) {
                                    if let Some(target_vk) = code_to_vk(code) {
                                        if vk_code == target_vk {
                                            // Check modifiers
                                            let ctrl_down = unsafe { GetAsyncKeyState(VK_CONTROL as i32) } < 0;
                                            let alt_down = unsafe { GetAsyncKeyState(VK_MENU as i32) } < 0;
                                            let shift_down = unsafe { GetAsyncKeyState(VK_SHIFT as i32) } < 0;
                                            let win_down = unsafe { GetAsyncKeyState(VK_LWIN as i32) } < 0 || unsafe { GetAsyncKeyState(VK_RWIN as i32) } < 0;
                                            
                                            let mods_match = 
                                                (!mods.ctrl || ctrl_down) &&
                                                (!mods.alt || alt_down) &&
                                                (!mods.shift || shift_down) &&
                                                (!mods.cmd || win_down);
                                            
                                            if mods_match {
                                                if let Some(state_arc) = STATE.get() {
                                                    if let Ok(guard) = state_arc.lock() {
                                                        if let Some(state) = guard.as_ref() {
                                                            if !is_up {
                                                                let was_pressed = {
                                                                    let mut pressed = state.key_pressed.lock().unwrap();
                                                                    let was = *pressed;
                                                                    *pressed = true;
                                                                    was
                                                                };
                                                                
                                                                if !was_pressed {
                                                                    eprintln!("[kiklet][hotkey] down kind=combo code={}", code);
                                                                    let _ = app.emit("hotkey:down", ());
                                                                }
                                                            } else {
                                                                let was_pressed = {
                                                                    let mut pressed = state.key_pressed.lock().unwrap();
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
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    unsafe {
        CallNextHookEx(0, n_code, w_param, l_param)
    }
}

fn message_loop_thread(app: AppHandle, config: HotkeyConfig, stop_flag: Arc<Mutex<bool>>) {
    unsafe {
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            keyboard_hook_proc,
            std::ptr::null(),
            0,
        );
        
        if hook == 0 {
            eprintln!("[kiklet][hotkey_tracker][win] SetWindowsHookExW failed");
            return;
        }
        
        if let Some(state_arc) = STATE.get() {
            if let Ok(mut guard) = state_arc.lock() {
                if let Some(ref mut state) = guard.as_mut() {
                    state.hook = hook;
                }
            }
        }
        
        let mut msg = MSG {
            hwnd: std::ptr::null(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        
        loop {
            if let Ok(stop) = stop_flag.lock() {
                if *stop {
                    break;
                }
            }
            
            let ret = GetMessageW(&mut msg, std::ptr::null(), 0, 0);
            if ret <= 0 {
                break;
            }
            
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        
        UnhookWindowsHookEx(hook);
    }
}

pub fn start(app: &AppHandle, config: HotkeyConfig) -> Result<(), String> {
    let _ = stop(app);
    
    eprintln!("[kiklet][hotkey_tracker][win] start kind={}", config.kind);
    
    let stop_flag = Arc::new(Mutex::new(false));
    let pressed = Arc::new(Mutex::new(false));
    let key_pressed = Arc::new(Mutex::new(false));
    
    HOOK_USER_DATA
        .set(Arc::new(Mutex::new(Some((config.clone(), app.clone())))))
        .map_err(|_| "hook user data already set".to_string())?;
    
    let app_clone = app.clone();
    let stop_flag_thread = stop_flag.clone();
    let thread_handle = thread::spawn(move || {
        message_loop_thread(app_clone, config.clone(), stop_flag_thread);
    });
    
    let state = TrackerState {
        config,
        app: app.clone(),
        hook: 0,
        thread: Some(thread_handle),
        stop_flag,
        pressed,
        key_pressed,
    };
    
    STATE
        .set(Arc::new(Mutex::new(Some(state))))
        .map_err(|_| "state already set".to_string())?;
    
    eprintln!("[kiklet][hotkey_tracker][win] start ok");
    Ok(())
}

pub fn stop(_app: &AppHandle) -> Result<(), String> {
    if let Some(state_arc) = STATE.get() {
        if let Ok(mut guard) = state_arc.lock() {
            if let Some(state) = guard.take() {
                eprintln!("[kiklet][hotkey_tracker][win] stop");
                
                if let Ok(mut stop) = state.stop_flag.lock() {
                    *stop = true;
                }
                
                unsafe {
                    PostThreadMessageW(0, WM_QUIT, 0, 0);
                }
                
                if state.hook != 0 {
                    unsafe {
                        UnhookWindowsHookEx(state.hook);
                    }
                }
                
                if let Some(thread) = state.thread {
                    let _ = thread.join();
                }
                
                if let Some(data_arc) = HOOK_USER_DATA.get() {
                    if let Ok(mut guard) = data_arc.lock() {
                        *guard = None;
                    }
                }
                
                eprintln!("[kiklet][hotkey_tracker][win] stop ok");
            }
        }
    }
    Ok(())
}

