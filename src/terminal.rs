use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::Command;

#[derive(Debug, Clone)]
pub enum TerminalKind {
    ITerm,
    TerminalApp,
    Warp,
    Tmux,
    Zellij,
    Custom(String),
    Unknown,
}

/// Detect terminal from environment variables.
/// Multiplexers checked first — they run inside other terminal emulators.
pub fn detect_terminal() -> TerminalKind {
    if std::env::var("ZELLIJ").is_ok() {
        return TerminalKind::Zellij;
    }
    if std::env::var("TMUX").is_ok() {
        return TerminalKind::Tmux;
    }
    match std::env::var("TERM_PROGRAM").as_deref() {
        Ok("iTerm.app") => TerminalKind::ITerm,
        Ok("Apple_Terminal") => TerminalKind::TerminalApp,
        Ok("WarpTerminal") => TerminalKind::Warp,
        _ => TerminalKind::Unknown,
    }
}

/// Load terminal preference from ~/.claude/stats-config.json
fn load_terminal_config() -> Option<TerminalKind> {
    let home = dirs::home_dir()?;
    let path = home.join(".claude").join("stats-config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: Value = serde_json::from_str(&content).ok()?;
    let terminal = val.get("terminal")?;

    // String value: "iterm", "terminal_app", etc.
    if let Some(s) = terminal.as_str() {
        return match s {
            "iterm" => Some(TerminalKind::ITerm),
            "terminal_app" => Some(TerminalKind::TerminalApp),
            "warp" => Some(TerminalKind::Warp),
            "tmux" => Some(TerminalKind::Tmux),
            "zellij" => Some(TerminalKind::Zellij),
            "auto" => None,
            _ => None,
        };
    }

    // Object value: {"type": "custom", "command": "..."}
    if let Some(obj) = terminal.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("custom") {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                return Some(TerminalKind::Custom(cmd.to_string()));
            }
        }
    }

    None
}

/// Config override > auto-detect
pub fn resolve_terminal() -> TerminalKind {
    if let Some(kind) = load_terminal_config() {
        return kind;
    }
    detect_terminal()
}

fn shell_escape(s: &str) -> String {
    // Single-quote escape: wrap in single quotes, escape internal single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open a claude --resume command in a new terminal tab/window.
pub fn open_in_new_tab(session_id: &str, cwd: &str) -> Result<(), String> {
    let kind = resolve_terminal();
    let cmd = format!("cd {} && claude --resume {}", shell_escape(cwd), session_id);

    match kind {
        TerminalKind::ITerm => {
            let script = format!(
                r#"tell application "iTerm"
    if (count of windows) = 0 then
        create window with default profile
    else
        tell current window
            create tab with default profile
        end tell
    end if
    tell current session of current window
        write text "{}"
    end tell
end tell"#,
                cmd.replace('"', "\\\"")
            );
            run_osascript(&script)
        }
        TerminalKind::TerminalApp => {
            let script = format!(
                r#"tell application "Terminal"
    do script "{}"
    activate
end tell"#,
                cmd.replace('"', "\\\"")
            );
            run_osascript(&script)
        }
        TerminalKind::Warp => {
            // Warp responds to the same AppleScript pattern as Terminal.app
            let script = format!(
                r#"tell application "Warp"
    do script "{}"
    activate
end tell"#,
                cmd.replace('"', "\\\"")
            );
            match run_osascript(&script) {
                Ok(()) => Ok(()),
                Err(_) => Err("Warp scripting not supported. Run `claude-stats --config-terminal` to set a custom command.".into()),
            }
        }
        TerminalKind::Tmux => {
            let output = Command::new("tmux")
                .args(["new-window", &cmd])
                .output()
                .map_err(|e| format!("tmux: {}", e))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!("tmux: {}", String::from_utf8_lossy(&output.stderr)))
            }
        }
        TerminalKind::Zellij => {
            let output = Command::new("zellij")
                .args(["action", "new-tab", "--", "bash", "-c", &cmd])
                .output()
                .map_err(|e| format!("zellij: {}", e))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!("zellij: {}", String::from_utf8_lossy(&output.stderr)))
            }
        }
        TerminalKind::Custom(template) => {
            let full_cmd = template.replace("{cmd}", &cmd);
            let output = Command::new("sh")
                .args(["-c", &full_cmd])
                .output()
                .map_err(|e| format!("custom: {}", e))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!("custom: {}", String::from_utf8_lossy(&output.stderr)))
            }
        }
        TerminalKind::Unknown => {
            Err("Terminal not detected. Press C to open here, or run `claude-stats --config-terminal`.".into())
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    #[allow(dead_code)]
    pub pid: u32,
    pub tty: String,
}

/// Get the TTY of the current process (for self-detection).
pub fn current_tty() -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "tty=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if tty.is_empty() || tty == "??" { None } else { Some(tty) }
}

/// Scan running claude processes and map session_id → ProcessInfo.
///
/// Two strategies:
/// 1. `--resume <id>` in args → direct session ID extraction
/// 2. Bare `claude` processes → match CWD to session file paths
///
/// The `sessions` parameter provides known sessions for CWD matching:
/// `(session_id, file_path, end_ts_epoch)` sorted most-recent-first.
pub fn scan_claude_processes(sessions: &[(String, String, i64)]) -> HashMap<String, ProcessInfo> {
    let mut map = HashMap::new();

    // Step 1: Find all `claude` processes with a TTY
    let output = match Command::new("ps")
        .args(["-eo", "pid,tty,comm,args"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return map,
    };

    let mut claude_pids: Vec<(u32, String, String)> = Vec::new(); // (pid, tty, args)
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        // Split on whitespace runs (handles variable padding from ps)
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 4 {
            continue;
        }
        let pid: u32 = match tokens[0].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let tty = tokens[1];
        let comm = tokens[2];
        // args is everything from token 3 onward (rejoined)
        let args = tokens[3..].join(" ");

        // Only actual claude processes with a real TTY
        if comm != "claude" || tty == "??" || tty == "-" {
            continue;
        }

        // Strategy 1: --resume <id> in args
        if let Some(pos) = args.find("--resume") {
            let after = &args[pos + "--resume".len()..].trim_start();
            if let Some(session_id) = after.split_whitespace().next() {
                if !session_id.is_empty() {
                    map.insert(session_id.to_string(), ProcessInfo { pid, tty: tty.to_string() });
                    continue;
                }
            }
        }

        // Collect for CWD-based matching
        claude_pids.push((pid, tty.to_string(), args));
    }

    // Step 2: For bare `claude` processes, get CWDs via lsof
    if !claude_pids.is_empty() && !sessions.is_empty() {
        let pid_args: Vec<String> = claude_pids.iter().map(|(pid, _, _)| format!("-p{}", pid)).collect();
        let mut lsof_args = vec!["-d".to_string(), "cwd".to_string(), "-Fn".to_string()];
        lsof_args.extend(pid_args);

        if let Ok(output) = Command::new("lsof").args(&lsof_args).output() {
            let lsof_out = String::from_utf8_lossy(&output.stdout);
            let mut current_pid: Option<u32> = None;

            for line in lsof_out.lines() {
                if let Some(pid_str) = line.strip_prefix('p') {
                    current_pid = pid_str.parse().ok();
                } else if let Some(path) = line.strip_prefix('n') {
                    if let Some(pid) = current_pid {
                        // Find the matching claude_pid entry
                        if let Some((_, tty, _)) = claude_pids.iter().find(|(p, _, _)| *p == pid) {
                            // Match CWD to the most recent session from that project.
                            // Sessions are sorted most-recent-first, so first match wins.
                            // Strip .claude/worktrees/<name> suffix — worktree CWDs
                            // won't match the project-root-based session paths otherwise.
                            let normalized = if let Some(idx) = path.find("/.claude/worktrees/") {
                                &path[..idx]
                            } else {
                                path
                            };
                            let encoded_cwd = normalized.replace('/', "-");
                            if let Some((session_id, _, _)) = sessions.iter()
                                .find(|(sid, fp, _)| fp.contains(&encoded_cwd) && !map.contains_key(sid))
                            {
                                map.insert(session_id.clone(), ProcessInfo { pid, tty: tty.clone() });
                            }
                        }
                    }
                }
            }
        }
    }

    map
}

/// Focus an existing iTerm2 tab by matching its TTY.
/// Returns Ok if the tab was found and focused, Err otherwise.
pub fn focus_tab_by_tty(tty: &str) -> Result<(), String> {
    let kind = resolve_terminal();
    match kind {
        TerminalKind::ITerm => {
            // ps gives "ttys003", iTerm reports "/dev/ttys003"
            let full_tty = if tty.starts_with("/dev/") {
                tty.to_string()
            } else {
                format!("/dev/{}", tty)
            };

            let script = format!(
                r#"tell application "iTerm"
    repeat with aWindow in windows
        repeat with aTab in tabs of aWindow
            repeat with aSession in sessions of aTab
                if (tty of aSession) = "{}" then
                    select aTab
                    tell aWindow to select
                    activate
                    return "found"
                end if
            end repeat
        end repeat
    end repeat
    return "not found"
end tell"#,
                full_tty
            );

            let output = Command::new("osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| format!("osascript: {}", e))?;

            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if result == "found" {
                Ok(())
            } else {
                Err("Tab not found in iTerm2".into())
            }
        }
        TerminalKind::Tmux => {
            // tmux: find pane by TTY and switch to it
            let output = Command::new("tmux")
                .args(["list-panes", "-a", "-F", "#{pane_tty} #{session_name}:#{window_index}"])
                .output()
                .map_err(|e| format!("tmux: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let full_tty = if tty.starts_with("/dev/") {
                tty.to_string()
            } else {
                format!("/dev/{}", tty)
            };

            for line in stdout.lines() {
                if line.starts_with(&full_tty) {
                    if let Some(target) = line.split_whitespace().nth(1) {
                        let result = Command::new("tmux")
                            .args(["select-window", "-t", target])
                            .output()
                            .map_err(|e| format!("tmux select: {}", e))?;
                        if result.status.success() {
                            return Ok(());
                        }
                    }
                }
            }
            Err("Pane not found in tmux".into())
        }
        _ => Err("Focus not supported for this terminal".into()),
    }
}

fn run_osascript(script: &str) -> Result<(), String> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("osascript: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("osascript: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

/// Interactive CLI flow to configure terminal preference.
pub fn run_config_terminal_flow() {
    println!("\n  Configure terminal for claude-stats\n");

    // Show auto-detection result
    let detected = detect_terminal();
    match &detected {
        TerminalKind::ITerm => println!("  Auto-detected: iTerm2"),
        TerminalKind::TerminalApp => println!("  Auto-detected: Terminal.app"),
        TerminalKind::Warp => println!("  Auto-detected: Warp"),
        TerminalKind::Tmux => println!("  Auto-detected: tmux"),
        TerminalKind::Zellij => println!("  Auto-detected: zellij"),
        _ => println!("  Could not auto-detect terminal"),
    }

    println!();
    println!("  Choose your terminal for opening new tabs:");
    println!("  1. iTerm2");
    println!("  2. Terminal.app");
    println!("  3. Warp");
    println!("  4. tmux");
    println!("  5. zellij");
    println!("  6. Custom command");
    println!("  7. Auto-detect (default)");
    println!();
    print!("  Choice [1-7]: ");
    io::stdout().flush().ok();

    let stdin = io::stdin();
    let choice = stdin.lock().lines().next()
        .and_then(|l| l.ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    let terminal_value: Value = match choice.as_str() {
        "1" => Value::String("iterm".into()),
        "2" => Value::String("terminal_app".into()),
        "3" => Value::String("warp".into()),
        "4" => Value::String("tmux".into()),
        "5" => Value::String("zellij".into()),
        "6" => {
            println!();
            println!("  Enter command template (use {{cmd}} as placeholder):");
            println!("  Example: my-terminal -e {{cmd}}");
            println!();
            print!("  Command: ");
            io::stdout().flush().ok();

            let custom = stdin.lock().lines().next()
                .and_then(|l| l.ok())
                .unwrap_or_default()
                .trim()
                .to_string();

            if custom.is_empty() {
                println!("  No command entered. Aborting.");
                return;
            }

            serde_json::json!({"type": "custom", "command": custom})
        }
        _ => Value::String("auto".into()),
    };

    // Read existing config, merge terminal key, write back
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("  Could not determine home directory.");
            return;
        }
    };
    let config_path = home.join(".claude").join("stats-config.json");
    let mut config: Value = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(obj) = config.as_object_mut() {
        obj.insert("terminal".into(), terminal_value.clone());
    }

    match std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()) {
        Ok(()) => {
            println!();
            println!("  Saved to {}", config_path.display());
            let label = match &terminal_value {
                Value::String(s) => s.clone(),
                _ => "custom".into(),
            };
            println!("  Terminal set to: {}", label);
        }
        Err(e) => eprintln!("  Failed to write config: {}", e),
    }
}
