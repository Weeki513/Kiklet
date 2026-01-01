use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use super::PttConfig;

static GEN: AtomicU64 = AtomicU64::new(0);

// FFI bindings for ApplicationServices
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

// CGEventSourceStateID constants
const K_CGEVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;
const K_CGEVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;

// Map code to macOS virtual keycode
fn code_to_vk(code: &str) -> Option<u16> {
    match code {
        "KeyA" => Some(0), "KeyS" => Some(1), "KeyD" => Some(2), "KeyF" => Some(3),
        "KeyH" => Some(4), "KeyG" => Some(5), "KeyZ" => Some(6), "KeyX" => Some(7),
        "KeyC" => Some(8), "KeyV" => Some(9), "KeyB" => Some(11), "KeyQ" => Some(12),
        "KeyW" => Some(13), "KeyE" => Some(14), "KeyR" => Some(15), "KeyY" => Some(16),
        "KeyT" => Some(17), "Key1" => Some(18), "Key2" => Some(19), "Key3" => Some(20),
        "Key4" => Some(21), "Key6" => Some(22), "Key5" => Some(23), "Key9" => Some(25),
        "Key7" => Some(26), "Key8" => Some(28), "Key0" => Some(29), "KeyO" => Some(31),
        "KeyU" => Some(32), "KeyI" => Some(34), "KeyP" => Some(35), "Enter" => Some(36),
        "KeyL" => Some(37), "KeyJ" => Some(38), "KeyK" => Some(40), "KeyN" => Some(45),
        "KeyM" => Some(46), "Tab" => Some(48), "Space" => Some(49), "Backspace" => Some(51),
        "Escape" => Some(53), "F1" => Some(122), "F2" => Some(120), "F3" => Some(99),
        "F4" => Some(118), "F5" => Some(96), "F6" => Some(97), "F7" => Some(98),
        "F8" => Some(100), "F9" => Some(101), "F10" => Some(109), "F11" => Some(103),
        "F12" => Some(111), "F13" => Some(105), "F14" => Some(107), "F15" => Some(113),
        "F16" => Some(106), "F17" => Some(64), "F18" => Some(79), "F19" => Some(80),
        "F20" => Some(90), "F21" => Some(83), "F22" => Some(84), "F23" => Some(85),
        "F24" => Some(86), "ArrowUp" => Some(126), "ArrowDown" => Some(125),
        "ArrowLeft" => Some(123), "ArrowRight" => Some(124), "PageUp" => Some(116),
        "PageDown" => Some(121), "Home" => Some(115), "End" => Some(119),
        "Insert" => Some(114), "Delete" => Some(117),
        _ => {
            if code.starts_with("Digit") {
                let num = code.strip_prefix("Digit")?;
                num.parse::<u16>().ok().map(|n| n + 18)
            } else if code.starts_with("Key") {
                let letter = code.strip_prefix("Key")?;
                if letter.len() == 1 {
                    let ch = letter.chars().next()?;
                    if ch.is_ascii_alphabetic() {
                        Some(ch.to_ascii_uppercase() as u16 - 65) // A=0, B=1, etc
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

struct PttState {
    target_vk: u16,
    app: AppHandle,
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

static STATE: OnceLock<Arc<Mutex<Option<PttState>>>> = OnceLock::new();

pub fn start(app: &AppHandle, config: PttConfig) -> Result<(), String> {
    eprintln!("[kiklet][ptt] start requested accelerator={} code={:?}", config.accelerator, config.code);
    
    // Get target keycode
    let target_vk = if let Some(code) = &config.code {
        if let Some(vk) = code_to_vk(code) {
            vk
        } else {
            return Err(format!("unknown key code: {}", code));
        }
    } else {
        return Err("missing key code in config".to_string());
    };
    
    // Check if already running with same target_vk (idempotent)
    if let Some(state_arc) = STATE.get() {
        if let Ok(guard) = state_arc.lock() {
            if let Some(state) = guard.as_ref() {
                if state.target_vk == target_vk && state.running.load(Ordering::Relaxed) {
                    eprintln!("[kiklet][ptt] start noop (already running same target)");
                    return Ok(());
                }
            }
        }
    }
    
    // Stop existing tracker if running (different target_vk or not running)
    let _ = stop(app);
    
    // Increment generation
    let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;
    
    eprintln!("[kiklet][ptt] start running=true target_vk={} gen={}", target_vk, gen);
    
    let running = Arc::new(AtomicBool::new(true));
    let running_thread = running.clone();
    let app_clone = app.clone();
    let gen_thread = gen;
    
    let thread_handle = thread::spawn(move || {
        let mut was_pressed = false;
        
        while running_thread.load(Ordering::Relaxed) {
            // Check generation - exit if outdated
            let current_gen = GEN.load(Ordering::Relaxed);
            if current_gen != gen_thread {
                eprintln!("[kiklet][ptt][mac] polling thread outdated gen={} current={}, exiting", gen_thread, current_gen);
                break;
            }
            
            // Poll key state using HID system state
            let pressed = unsafe {
                CGEventSourceKeyState(K_CGEVENT_SOURCE_STATE_HID_SYSTEM_STATE, target_vk)
            };
            
            // Detect state changes
            if pressed && !was_pressed {
                eprintln!("[kiklet][ptt][mac] down gen={}", gen_thread);
                let _ = app_clone.emit("hotkey:down", ());
            } else if !pressed && was_pressed {
                eprintln!("[kiklet][ptt][mac] up gen={}", gen_thread);
                let _ = app_clone.emit("hotkey:up", ());
            }
            
            was_pressed = pressed;
            
            // Sleep 10ms between polls
            thread::sleep(Duration::from_millis(10));
        }
        
        eprintln!("[kiklet][ptt][mac] polling thread exited gen={}", gen_thread);
    });
    
    let state = PttState {
        target_vk,
        app: app.clone(),
        thread: Some(thread_handle),
        running: running.clone(),
    };
    
    STATE.set(Arc::new(Mutex::new(Some(state)))).map_err(|_| "state already set".to_string())?;
    
    eprintln!("[kiklet][ptt][mac] start ok");
    Ok(())
}

pub fn stop(_app: &AppHandle) -> Result<(), String> {
    if let Some(state_arc) = STATE.get() {
        if let Ok(mut guard) = state_arc.lock() {
            if let Some(state) = guard.take() {
                eprintln!("[kiklet][ptt][mac] stop");
                
                // Signal thread to stop
                state.running.store(false, Ordering::Relaxed);
                
                // Wait for thread to finish
                if let Some(thread) = state.thread {
                    if let Err(e) = thread.join() {
                        eprintln!("[kiklet][ptt][mac] thread join error: {:?}", e);
                    }
                }
                
                eprintln!("[kiklet][ptt][mac] stop ok");
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
