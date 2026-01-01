use std::process::Command;

use tauri::Manager;

const RUN_KEY: &str = r#"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"#;
const VALUE_NAME: &str = "Kiklet";

fn current_exe_quoted() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
    let s = exe
        .to_str()
        .ok_or_else(|| "invalid exe path".to_string())?
        .to_string();
    if s.contains(' ') {
        Ok(format!("\"{}\"", s))
    } else {
        Ok(s)
    }
}

fn reg(args: &[&str]) -> Result<(), String> {
    let out = Command::new("reg")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run reg {:?}: {e}", args))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "reg {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn reg_ok(args: &[&str]) -> bool {
    Command::new("reg")
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn enable(_app: &tauri::AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][autostart] enabling…");
    let exe = current_exe_quoted()?;
    reg(&[
        "add",
        RUN_KEY,
        "/v",
        VALUE_NAME,
        "/t",
        "REG_SZ",
        "/d",
        &exe,
        "/f",
    ])?;
    eprintln!("[kiklet][autostart] enabled ok");
    Ok(())
}

pub fn disable(_app: &tauri::AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][autostart] disabling…");
    // idempotent: treat missing value as ok
    if reg_ok(&["query", RUN_KEY, "/v", VALUE_NAME]) {
        let _ = reg(&["delete", RUN_KEY, "/v", VALUE_NAME, "/f"]);
    }
    eprintln!("[kiklet][autostart] disabled ok");
    Ok(())
}

pub fn is_enabled(_app: &tauri::AppHandle) -> Result<bool, String> {
    Ok(reg_ok(&["query", RUN_KEY, "/v", VALUE_NAME]))
}


