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

#[derive(Debug, Clone, PartialEq)]
pub enum MatchConfidence {
    Direct, // --resume arg or task dir open
    High,   // CWD match, unique session for that project
    Low,    // CWD match, multiple sessions share this CWD
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub tty: String,
    pub confidence: MatchConfidence,
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

/// Query iTerm2 for all tab TTYs and their display names.
/// Returns a map of TTY (e.g. "ttys003") → cleaned session title.
fn get_iterm_tab_names() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let script = r#"tell application "iTerm2"
    set output to ""
    repeat with w in windows
        repeat with t in tabs of w
            repeat with s in sessions of t
                set output to output & (tty of s) & "|||" & (name of s) & "
"
            end repeat
        end repeat
    end repeat
    return output
end tell"#;

    let output = match Command::new("osascript").arg("-e").arg(script).output() {
        Ok(o) if o.status.success() => o,
        _ => return map,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some((tty_raw, name)) = line.split_once("|||") else { continue };
        // TTY: "/dev/ttys003" → "ttys003"
        let tty = tty_raw.strip_prefix("/dev/").unwrap_or(tty_raw).to_string();
        if tty.is_empty() { continue; }
        // Clean name: strip spinner prefixes (✳, ⠐, ⠂, etc) and " (node)"/" (zsh)" suffix
        let clean = name
            .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '/')
            .trim_start()
            .trim_end_matches(" (node)")
            .trim_end_matches(" (zsh)")
            .trim_end_matches(" (bash)")
            .trim()
            .to_string();
        if !clean.is_empty() {
            map.insert(tty, clean);
        }
    }
    map
}

/// Per-PID info extracted from lsof output.
struct LsofPidInfo {
    cwd: Option<String>,
    task_uuid: Option<String>,
}

/// Scan running claude processes and map session_id → ProcessInfo.
///
/// Multi-signal confidence matching:
/// 1. `--resume <id>` in args → Direct
/// 2. iTerm2 tab name → session title match → Direct
/// 3. Task dir open `~/.claude/tasks/{uuid}` → Direct (uuid = session ID)
/// 4. CWD match, unique session → High
/// 5. CWD match, multiple sessions → Low (pick by most recent end_ts)
pub fn scan_claude_processes(sessions: &[(String, String, i64, String)]) -> HashMap<String, ProcessInfo> {
    let mut map = HashMap::new();

    // Step 1: Find all `claude` processes with a TTY
    let output = match Command::new("ps")
        .args(["-eo", "pid,tty,comm,args"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return map,
    };

    let mut claude_pids: Vec<(u32, String)> = Vec::new(); // (pid, tty)
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 4 { continue; }
        let pid: u32 = match tokens[0].parse() { Ok(p) => p, Err(_) => continue };
        let tty = tokens[1];
        let comm = tokens[2];
        let args = tokens[3..].join(" ");

        if comm != "claude" || tty == "??" || tty == "-" { continue; }

        // Strategy 1: --resume <id> in args → Direct
        if let Some(pos) = args.find("--resume") {
            let after = &args[pos + "--resume".len()..].trim_start();
            if let Some(session_id) = after.split_whitespace().next() {
                if !session_id.is_empty() {
                    map.insert(session_id.to_string(), ProcessInfo {
                        pid, tty: tty.to_string(), confidence: MatchConfidence::Direct,
                    });
                    continue;
                }
            }
        }

        claude_pids.push((pid, tty.to_string()));
    }

    // Strategy 2: iTerm2 tab name → session title matching
    if !claude_pids.is_empty() {
        let tab_names = get_iterm_tab_names(); // tty → cleaned tab title
        let mut matched_pids: Vec<u32> = Vec::new();

        for (pid, tty) in &claude_pids {
            if let Some(tab_title) = tab_names.get(tty.as_str()) {
                // Find a session whose title matches this tab name (case-insensitive)
                let tab_lower = tab_title.to_lowercase();
                if let Some((sid, _, _, _)) = sessions.iter()
                    .find(|(sid, _, _, title)| {
                        !title.is_empty() && title.to_lowercase() == tab_lower && !map.contains_key(sid)
                    })
                {
                    map.insert(sid.clone(), ProcessInfo {
                        pid: *pid, tty: tty.clone(), confidence: MatchConfidence::Direct,
                    });
                    matched_pids.push(*pid);
                }
            }
        }

        // Remove matched PIDs so they don't go through lsof fallback
        claude_pids.retain(|(pid, _)| !matched_pids.contains(pid));
    }

    // Step 2: For bare `claude` processes, get ALL open files via lsof
    if !claude_pids.is_empty() && !sessions.is_empty() {
        let pid_list = claude_pids.iter()
            .map(|(pid, _)| pid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let lsof_args = ["-a", "-Fn", "-d", "cwd,10-25", &format!("-p{}", pid_list)];

        if let Ok(output) = Command::new("lsof").args(lsof_args).output() {
            let lsof_out = String::from_utf8_lossy(&output.stdout);

            // Parse lsof output into per-PID info
            let mut pid_info: HashMap<u32, LsofPidInfo> = HashMap::new();
            let mut current_pid: Option<u32> = None;
            let mut is_cwd_fd = false;

            let tasks_prefix = dirs::home_dir()
                .map(|h| format!("{}/.claude/tasks/", h.display()))
                .unwrap_or_default();

            for line in lsof_out.lines() {
                if let Some(pid_str) = line.strip_prefix('p') {
                    current_pid = pid_str.parse().ok();
                    is_cwd_fd = false;
                    if let Some(pid) = current_pid {
                        pid_info.entry(pid).or_insert(LsofPidInfo {
                            cwd: None, task_uuid: None,
                        });
                    }
                } else if line.starts_with('f') {
                    is_cwd_fd = line == "fcwd";
                } else if let Some(path) = line.strip_prefix('n') {
                    if let Some(pid) = current_pid {
                        let info = pid_info.entry(pid).or_insert(LsofPidInfo {
                            cwd: None, task_uuid: None,
                        });

                        // CWD
                        if is_cwd_fd {
                            info.cwd = Some(path.to_string());
                            is_cwd_fd = false;
                        }

                        // Task dir: ~/.claude/tasks/{uuid}
                        if info.task_uuid.is_none() {
                            if let Some(rest) = path.strip_prefix(&tasks_prefix) {
                                // rest might be the UUID or UUID/subpath
                                let uuid = rest.split('/').next().unwrap_or("");
                                if !uuid.is_empty() && uuid.contains('-') {
                                    info.task_uuid = Some(uuid.to_string());
                                }
                            }
                        }
                    }
                }
            }

            // Step 3: Match in priority order
            let session_ids: std::collections::HashSet<&str> = sessions.iter()
                .map(|(sid, _, _, _)| sid.as_str())
                .collect();

            for (pid, tty) in &claude_pids {
                let Some(info) = pid_info.get(pid) else { continue };

                // Strategy 2: Task dir → Direct match
                if let Some(ref uuid) = info.task_uuid {
                    if session_ids.contains(uuid.as_str()) && !map.contains_key(uuid) {
                        map.insert(uuid.clone(), ProcessInfo {
                            pid: *pid, tty: tty.clone(), confidence: MatchConfidence::Direct,
                        });
                        continue;
                    }
                }

                // Strategy 3/4: CWD matching
                // Compare encoded CWD to the project directory name in the file path
                // exactly — not substring. File paths look like:
                //   ~/.claude/projects/{encoded_project_dir}/{session-id}.jsonl
                // Substring matching caused /Users/chrisjones to match every project.
                if let Some(ref cwd) = info.cwd {
                    let home = dirs::home_dir().unwrap_or_default();
                    if cwd == "/" || cwd.as_str() == home.to_string_lossy().as_ref() {
                        continue;
                    }
                    let normalized = if let Some(idx) = cwd.find("/.claude/worktrees/") {
                        &cwd[..idx]
                    } else {
                        cwd.as_str()
                    };
                    let encoded_cwd = normalized.replace('/', "-");

                    let matches: Vec<&(String, String, i64, String)> = sessions.iter()
                        .filter(|(sid, fp, _, _)| {
                            if map.contains_key(sid) { return false; }
                            let project_dir = std::path::Path::new(fp.as_str())
                                .parent()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            project_dir == encoded_cwd
                        })
                        .collect();

                    let confidence = if matches.len() == 1 {
                        MatchConfidence::High
                    } else {
                        MatchConfidence::Low
                    };

                    if let Some((session_id, _, _, _)) = matches.iter()
                        .max_by_key(|(_, _, end_ts, _)| *end_ts)
                    {
                        map.insert(session_id.clone(), ProcessInfo {
                            pid: *pid, tty: tty.clone(), confidence,
                        });
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
