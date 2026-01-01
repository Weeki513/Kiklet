use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use super::{DeliveryMode, DeliveryResult};

fn trunc200(s: &str) -> String {
    let mut out = s.replace('\n', " ").replace('\r', " ");
    if out.len() > 200 {
        out.truncate(200);
    }
    out
}

fn ps_capture(script: &str) -> Result<(i32, String, String), String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .map_err(|e| format!("failed to run powershell: {e}"))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn ps_with_stdin(script: &str, input: &[u8]) -> Result<i32, String> {
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn powershell: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input)
            .map_err(|e| format!("failed to write stdin to powershell: {e}"))?;
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for powershell: {e}"))?;
    if out.status.success() {
        Ok(out.status.code().unwrap_or(0))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

fn get_clipboard() -> Option<String> {
    ps_capture("Get-Clipboard -Raw")
        .ok()
        .and_then(|(code, out, _err)| if code == 0 { Some(out) } else { None })
}

fn set_clipboard(text: &str) -> Result<(), String> {
    // Read value from stdin to avoid quoting/escaping issues.
    ps_with_stdin("Set-Clipboard -Value ([Console]::In.ReadToEnd())", text.as_bytes())
        .map(|_| ())
        .map_err(|e| format!("Set-Clipboard failed: {e}"))
}

fn paste_ctrl_v() -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        const INPUT_KEYBOARD: u32 = 1;
        const KEYEVENTF_KEYUP: u32 = 0x0002;
        const VK_CONTROL: u16 = 0x11;
        const VK_V: u16 = 0x56;

        #[repr(C)]
        struct KEYBDINPUT {
            wVk: u16,
            wScan: u16,
            dwFlags: u32,
            time: u32,
            dwExtraInfo: usize,
        }

        #[repr(C)]
        struct INPUT {
            r#type: u32,
            ki: KEYBDINPUT,
        }

        #[link(name = "user32")]
        extern "system" {
            fn SendInput(cInputs: u32, pInputs: *const INPUT, cbSize: i32) -> u32;
        }

        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    wScan: 0,
                    dwFlags: 0,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                ki: KEYBDINPUT {
                    wVk: VK_V,
                    wScan: 0,
                    dwFlags: 0,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                ki: KEYBDINPUT {
                    wVk: VK_V,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        ];
        // tiny delays help some apps
        sleep(Duration::from_millis(10));
        let sent = SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        if sent == inputs.len() as u32 {
            Ok(())
        } else {
            Err(format!("SendInput sent {sent}/{}", inputs.len()))
        }
    }
    #[cfg(not(windows))]
    {
        Err("not_windows".to_string())
    }
}

pub fn deliver(text: &str, attempt_insert: bool) -> Result<DeliveryResult, String> {
    eprintln!(
        "[kiklet][deliver][windows] start len={} attempt_insert={}",
        text.len(),
        attempt_insert
    );
    let old = get_clipboard();
    eprintln!(
        "[kiklet][deliver][windows] get_clipboard {}",
        if old.is_some() { "ok" } else { "failed" }
    );

    match set_clipboard(text) {
        Ok(()) => eprintln!("[kiklet][deliver][windows] set_clipboard ok"),
        Err(e) => {
            eprintln!(
                "[kiklet][deliver][windows] set_clipboard failed: {}",
                trunc200(&e)
            );
            return Err(format!("clipboard_failed: {}", trunc200(&e)));
        }
    }

    if !attempt_insert {
        return Ok(DeliveryResult {
            mode: DeliveryMode::Copy,
            ok: true,
            detail: Some("copied".to_string()),
        });
    }

    match paste_ctrl_v() {
        Ok(()) => {
            eprintln!("[kiklet][deliver][windows] paste ok");
            // Insert success (heuristic). Restore old clipboard if we have it.
            if let Some(old) = old {
                match set_clipboard(&old) {
                    Ok(()) => eprintln!("[kiklet][deliver][windows] clipboard restored ok"),
                    Err(e) => eprintln!(
                        "[kiklet][deliver][windows] clipboard restore failed: {}",
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
        Err(err) => {
            eprintln!(
                "[kiklet][deliver][windows] paste failed: {}",
                trunc200(&err)
            );
            // Paste failed: keep our clipboard value (Copy mode). Do NOT restore.
            Ok(DeliveryResult {
                mode: DeliveryMode::Copy,
                ok: true,
                detail: Some("paste_failed_copied".to_string()),
            })
        }
    }
}


