use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use super::{DeliveryMode, DeliveryResult};

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
    eprintln!(
        "[kiklet][deliver][macos] start len={} attempt_insert={}",
        text.len(),
        attempt_insert
    );
    // Best-effort: always set clipboard to text first.
    let old_clip = match run_capture("pbpaste", &[]) {
        Ok((code, out, err)) if code == 0 => {
            eprintln!("[kiklet][deliver][macos] pbpaste ok");
            Some(out)
        }
        Ok((code, _out, err)) => {
            eprintln!(
                "[kiklet][deliver][macos] pbpaste failed code={} stderr='{}'",
                code,
                trunc200(&err)
            );
            None
        }
        _ => None,
    };

    // Put our text into clipboard
    match run_with_stdin("pbcopy", &[], text.as_bytes()) {
        Ok(_) => eprintln!("[kiklet][deliver][macos] pbcopy ok"),
        Err(e) => {
            eprintln!("[kiklet][deliver][macos] pbcopy failed: {}", trunc200(&e));
            return Err(format!("clipboard_failed: {}", trunc200(&e)));
        }
    }

    if !attempt_insert {
        // Copy-only mode: keep our clipboard value (do not restore).
        return Ok(DeliveryResult {
            mode: DeliveryMode::Copy,
            ok: true,
            detail: Some("copied".to_string()),
        });
    }

    // Need Accessibility to post events.
    let trusted = crate::perm::macos::ax_is_trusted();
    eprintln!("[kiklet][deliver][macos] ax_trusted={}", trusted);
    if !trusted {
        return Ok(DeliveryResult {
            mode: DeliveryMode::Copy,
            ok: true,
            detail: Some("need_accessibility".to_string()),
        });
    }

    match try_paste_cmd_v() {
        Ok(()) => {
            eprintln!("[kiklet][deliver][macos] paste ok");
            // Restore old clipboard if we have it.
            if let Some(old) = old_clip {
                match run_with_stdin("pbcopy", &[], old.as_bytes()) {
                    Ok(_) => eprintln!("[kiklet][deliver][macos] clipboard restored ok"),
                    Err(e) => eprintln!(
                        "[kiklet][deliver][macos] clipboard restore failed: {}",
                        trunc200(&e)
                    ),
                }
            }
            Ok(DeliveryResult {
                mode: DeliveryMode::Insert,
                ok: true,
                detail: Some("inserted".to_string()),
            })
        }
        Err(e) => {
            eprintln!(
                "[kiklet][deliver][macos] paste failed: {}",
                trunc200(&e)
            );
            // Fallback: keep our clipboard (do not restore).
            Ok(DeliveryResult {
                mode: DeliveryMode::Copy,
                ok: true,
                detail: Some("paste_failed_copied".to_string()),
            })
        }
    }
}


