use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use tauri::{AppHandle, Emitter};

use super::ModifierKey;

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
    fn CGEventGetFlags(event: *const c_void) -> u64;
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
const kCGEventFlagsChanged: u64 = 1 << 11;

const kCGEventFlagMaskCommand: u64 = 1 << 1;
const kCGEventFlagMaskAlternate: u64 = 1 << 3;
const kCGEventFlagMaskShift: u64 = 1 << 17;
const kCGEventFlagMaskControl: u64 = 1 << 18;

// kCFRunLoopDefaultMode is NULL pointer
fn k_cf_run_loop_default_mode() -> *const c_void {
    std::ptr::null()
}

struct ModifierState {
    modifier: ModifierKey,
    app: AppHandle,
    thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
    last_flags: Arc<Mutex<u64>>,
    // Raw pointers stored as usize (safe for storage)
    event_tap: usize,
    run_loop: usize,
    source: usize,
}

static STATE: OnceLock<Arc<Mutex<Option<ModifierState>>>> = OnceLock::new();

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
        
        // Get state from user_info
        let state_ptr = user_info as *const Arc<Mutex<Option<(ModifierKey, AppHandle, Arc<Mutex<bool>>, Arc<Mutex<u64>>)>>>;
        let state_arc = &*state_ptr;
        
        if let Ok(guard) = state_arc.lock() {
            if let Some((modifier, app, stop_flag, last_flags)) = guard.as_ref() {
                // Check stop flag
                if let Ok(stop) = stop_flag.lock() {
                    if *stop {
                        return event;
                    }
                }
                
                // Get current flags
                let flags = CGEventGetFlags(event);
                
                // Get last flags and update
                let was_pressed = {
                    let mut last = last_flags.lock().unwrap();
                    let was = match modifier {
                        ModifierKey::Cmd => (*last & kCGEventFlagMaskCommand) != 0,
                        ModifierKey::Option => (*last & kCGEventFlagMaskAlternate) != 0,
                        ModifierKey::Shift => (*last & kCGEventFlagMaskShift) != 0,
                        ModifierKey::Ctrl => (*last & kCGEventFlagMaskControl) != 0,
                        ModifierKey::Win => false, // Not applicable on macOS
                    };
                    *last = flags;
                    was
                };
                
                // Check if modifier is now pressed
                let is_pressed = match modifier {
                    ModifierKey::Cmd => (flags & kCGEventFlagMaskCommand) != 0,
                    ModifierKey::Option => (flags & kCGEventFlagMaskAlternate) != 0,
                    ModifierKey::Shift => (flags & kCGEventFlagMaskShift) != 0,
                    ModifierKey::Ctrl => (flags & kCGEventFlagMaskControl) != 0,
                    ModifierKey::Win => false,
                };
                
                // Trigger on edge OFF->ON
                if is_pressed && !was_pressed {
                    eprintln!("[kiklet][mhotkey] trigger modifier={:?}", modifier);
                    let _ = app.emit("hotkey:toggle-record", ());
                }
            }
        }
    }
    event
}

pub fn start(app: &AppHandle, modifier: ModifierKey) -> Result<(), String> {
    // Stop existing if any
    let _ = stop(app);
    
    eprintln!("[kiklet][mhotkey][mac] start modifier={:?}", modifier);
    
    unsafe {
        // Create event tap
        let event_tap = CGEventTapCreate(
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            kCGEventTapOptionListenOnly,
            kCGEventFlagsChanged,
            event_tap_callback,
            std::ptr::null(),
        );
        
        if event_tap.is_null() {
            return Err("failed to create event tap (need Accessibility permission?)".to_string());
        }
        
        // Enable tap
        CGEventTapEnable(event_tap, true);
        
        // Create run loop source
        let source = CFMachPortCreateRunLoopSource(
            std::ptr::null(),
            event_tap,
            0,
        );
        
        if source.is_null() {
            CFRelease(event_tap);
            return Err("failed to create run loop source".to_string());
        }
        
        // Get current run loop
        let run_loop = CFRunLoopGetCurrent();
        
        // Add source to run loop
        CFRunLoopAddSource(run_loop, source, k_cf_run_loop_default_mode());
        
        // Create state for callback
        let stop_flag = Arc::new(Mutex::new(false));
        let last_flags = Arc::new(Mutex::new(0u64));
        
        // Store state pointer for callback
        let state_arc = Arc::new(Mutex::new(Some((
            modifier,
            app.clone(),
            stop_flag.clone(),
            last_flags.clone(),
        ))));
        let state_ptr = Arc::into_raw(state_arc.clone()) as *const c_void;
        
        // Re-create event tap with state pointer
        CFRelease(event_tap);
        let event_tap2 = CGEventTapCreate(
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            kCGEventTapOptionListenOnly,
            kCGEventFlagsChanged,
            event_tap_callback,
            state_ptr,
        );
        
        if event_tap2.is_null() {
            CFRelease(source);
            return Err("failed to create event tap with user info (need Accessibility permission?)".to_string());
        }
        
        CGEventTapEnable(event_tap2, true);
        
        // Start run loop in separate thread
        let run_loop_thread = run_loop;
        let _stop_flag_thread = stop_flag.clone();
        let app_clone = app.clone();
        let thread_handle = thread::spawn(move || {
            unsafe {
                CFRunLoopRun();
            }
        });
        
        // Create final state with raw pointers as usize
        let state = ModifierState {
            modifier,
            app: app_clone,
            thread: Some(thread_handle),
            stop_flag,
            last_flags,
            event_tap: event_tap2 as usize,
            run_loop: run_loop_thread as usize,
            source: source as usize,
        };
        
        // Store in global state
        STATE.set(Arc::new(Mutex::new(Some(state)))).map_err(|_| "state already set".to_string())?;
        
        eprintln!("[kiklet][mhotkey][mac] start ok");
        Ok(())
    }
}

pub fn stop(_app: &AppHandle) -> Result<(), String> {
    if let Some(state_arc) = STATE.get() {
        if let Ok(mut guard) = state_arc.lock() {
            if let Some(state) = guard.take() {
                eprintln!("[kiklet][mhotkey][mac] stop");
                
                // Set stop flag
                if let Ok(mut stop) = state.stop_flag.lock() {
                    *stop = true;
                }
                
                // Stop run loop and cleanup
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
                
                // Wait for thread
                if let Some(thread) = state.thread {
                    let _ = thread.join();
                }
                
                eprintln!("[kiklet][mhotkey][mac] stop ok");
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
