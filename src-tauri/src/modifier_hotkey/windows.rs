use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use tauri::{AppHandle, Emitter};

use super::ModifierKey;

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

struct ModifierState {
    modifier: ModifierKey,
    app: AppHandle,
    hook: isize,
    thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
    pressed: Arc<Mutex<bool>>,
}

static STATE: OnceLock<Arc<Mutex<Option<ModifierState>>>> = OnceLock::new();
static HOOK_USER_DATA: OnceLock<Arc<Mutex<Option<(ModifierKey, AppHandle)>>>> = OnceLock::new();

extern "system" fn keyboard_hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
    if n_code >= 0 {
        unsafe {
            let hook_struct = *(l_param as *const KBDLLHOOKSTRUCT);
            let is_up = (hook_struct.flags & LLKHF_UP) != 0;
            let vk_code = hook_struct.vk_code;
            
            // Get state
            if let Some(state_arc) = HOOK_USER_DATA.get() {
                if let Ok(guard) = state_arc.lock() {
                    if let Some((modifier, app)) = guard.as_ref() {
                        // Check if this is our modifier
                        let is_our_modifier = match modifier {
                            ModifierKey::Cmd | ModifierKey::Win => vk_code == VK_LWIN || vk_code == VK_RWIN,
                            ModifierKey::Option => vk_code == VK_MENU,
                            ModifierKey::Shift => vk_code == VK_SHIFT,
                            ModifierKey::Ctrl => vk_code == VK_CONTROL,
                        };
                        
                        if is_our_modifier && !is_up {
                            // Key down - check if it was already pressed
                            if let Some(state_arc) = STATE.get() {
                                if let Ok(guard) = state_arc.lock() {
                                    if let Some(state) = guard.as_ref() {
                                        let was_pressed = {
                                            let mut pressed = state.pressed.lock().unwrap();
                                            let was = *pressed;
                                            *pressed = true;
                                            was
                                        };
                                        
                                        // Trigger on edge OFF->ON
                                        if !was_pressed {
                                            eprintln!("[kiklet][mhotkey] trigger modifier={:?}", modifier);
                                            let _ = app.emit("hotkey:toggle-record", ());
                                        }
                                    }
                                }
                            }
                        } else if is_our_modifier && is_up {
                            // Key up - reset pressed state
                            if let Some(state_arc) = STATE.get() {
                                if let Ok(guard) = state_arc.lock() {
                                    if let Some(state) = guard.as_ref() {
                                        if let Ok(mut pressed) = state.pressed.lock() {
                                            *pressed = false;
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
    
    unsafe {
        CallNextHookEx(0, n_code, w_param, l_param)
    }
}

fn message_loop_thread(app: AppHandle, modifier: ModifierKey, stop_flag: Arc<Mutex<bool>>) {
    unsafe {
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            keyboard_hook_proc,
            std::ptr::null(),
            0,
        );
        
        if hook == 0 {
            eprintln!("[kiklet][mhotkey][win] SetWindowsHookExW failed");
            return;
        }
        
        // Store hook in state
        if let Some(state_arc) = STATE.get() {
            if let Ok(mut guard) = state_arc.lock() {
                if let Some(ref mut state) = guard.as_mut() {
                    state.hook = hook;
                }
            }
        }
        
        // Message loop
        let mut msg = MSG {
            hwnd: std::ptr::null(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        
        loop {
            // Check stop flag
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

pub fn start(app: &AppHandle, modifier: ModifierKey) -> Result<(), String> {
    // Stop existing if any
    let _ = stop(app);
    
    eprintln!("[kiklet][mhotkey][win] start modifier={:?}", modifier);
    
    let stop_flag = Arc::new(Mutex::new(false));
    let pressed = Arc::new(Mutex::new(false));
    
    // Store user data for hook
    HOOK_USER_DATA
        .set(Arc::new(Mutex::new(Some((modifier, app.clone())))))
        .map_err(|_| "hook user data already set".to_string())?;
    
    // Start message loop thread
    let app_clone = app.clone();
    let stop_flag_thread = stop_flag.clone();
    let thread_handle = thread::spawn(move || {
        message_loop_thread(app_clone, modifier, stop_flag_thread);
    });
    
    // Create state
    let state = ModifierState {
        modifier,
        app: app.clone(),
        hook: 0,
        thread: Some(thread_handle),
        stop_flag,
        pressed,
    };
    
    // Store state
    STATE
        .set(Arc::new(Mutex::new(Some(state))))
        .map_err(|_| "state already set".to_string())?;
    
    eprintln!("[kiklet][mhotkey][win] start ok");
    Ok(())
}

pub fn stop(_app: &AppHandle) -> Result<(), String> {
    if let Some(state_arc) = STATE.get() {
        if let Ok(mut guard) = state_arc.lock() {
            if let Some(state) = guard.take() {
                eprintln!("[kiklet][mhotkey][win] stop");
                
                // Set stop flag
                if let Ok(mut stop) = state.stop_flag.lock() {
                    *stop = true;
                }
                
                // Post quit message to thread
                if let Some(thread) = &state.thread {
                    // Get thread ID (simplified - in real impl might need to store it)
                    unsafe {
                        PostThreadMessageW(0, WM_QUIT, 0, 0);
                    }
                }
                
                // Unhook
                if state.hook != 0 {
                    unsafe {
                        UnhookWindowsHookEx(state.hook);
                    }
                }
                
                // Wait for thread
                if let Some(thread) = state.thread {
                    let _ = thread.join();
                }
                
                // Clear user data
                if let Some(data_arc) = HOOK_USER_DATA.get() {
                    if let Ok(mut guard) = data_arc.lock() {
                        *guard = None;
                    }
                }
                
                eprintln!("[kiklet][mhotkey][win] stop ok");
            }
        }
    }
    Ok(())
}

pub fn status(_app: &AppHandle) -> bool {
    if let Some(state_arc) = STATE.get() {
        if let Ok(guard) = state_arc.lock() {
            return guard.is_some();
        }
    }
    false
}
