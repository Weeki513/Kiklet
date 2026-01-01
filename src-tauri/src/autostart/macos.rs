use std::path::PathBuf;
use std::process::Command;

fn bundle_id(app: &tauri::AppHandle) -> String {
    app.config().identifier.clone()
}

fn label(app: &tauri::AppHandle) -> String {
    format!("{}.kiklet", bundle_id(app))
}

fn plist_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let mut p = PathBuf::from(home);
    p.push("Library");
    p.push("LaunchAgents");
    std::fs::create_dir_all(&p).map_err(|e| format!("failed to create LaunchAgents: {e}"))?;
    p.push(format!("{}.plist", label(app)));
    Ok(p)
}

fn uid() -> Result<String, String> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| format!("failed to run id -u: {e}"))?;
    if !out.status.success() {
        return Err("failed to get uid".to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run launchctl {:?}: {e}", args))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "launchctl {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn launchctl_ok(args: &[&str]) -> bool {
    Command::new("launchctl")
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write_plist(app: &tauri::AppHandle, path: &PathBuf) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
    let lbl = label(app);
    let exe_str = exe.to_string_lossy();

    if cfg!(debug_assertions) {
        eprintln!(
            "[kiklet][autostart] warning: enabling autostart in dev may be unstable (exe={})",
            exe_str
        );
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key><string>{label}</string>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
    <key>ProgramArguments</key>
    <array>
      <string>{exe}</string>
    </array>
  </dict>
</plist>
"#,
        label = lbl,
        exe = exe_str
    );

    std::fs::write(path, plist).map_err(|e| format!("failed to write plist: {e}"))?;
    Ok(())
}

pub fn enable(app: &tauri::AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][autostart] enabling…");

    let path = plist_path(app)?;
    write_plist(app, &path)?;

    let uid = uid()?;
    let gui = format!("gui/{uid}");

    // bootout may fail if not installed yet; ignore
    let _ = launchctl(&["bootout", &gui, path.to_string_lossy().as_ref()]);
    launchctl(&["bootstrap", &gui, path.to_string_lossy().as_ref()])?;

    eprintln!("[kiklet][autostart] enabled ok");
    Ok(())
}

pub fn disable(app: &tauri::AppHandle) -> Result<(), String> {
    eprintln!("[kiklet][autostart] disabling…");

    let path = plist_path(app)?;
    let uid = uid()?;
    let gui = format!("gui/{uid}");

    // bootout may fail if not loaded; treat as ok
    let _ = launchctl(&["bootout", &gui, path.to_string_lossy().as_ref()]);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("failed to remove plist: {e}"))?;
    }

    eprintln!("[kiklet][autostart] disabled ok");
    Ok(())
}

pub fn is_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    let path = plist_path(app)?;
    if !path.exists() {
        return Ok(false);
    }
    let uid = uid()?;
    let gui = format!("gui/{uid}/{}", label(app));
    if launchctl_ok(&["print", &gui]) {
        return Ok(true);
    }
    // fallback: plist exists
    Ok(true)
}


