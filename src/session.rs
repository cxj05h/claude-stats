use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum WaitingState {
    None,
    WaitingForInput,      // 👋 end_turn, no user text response yet
    WaitingForPermission, // ⏳ tool_use, no tool_result yet
}

#[derive(Debug, Clone)]
pub struct ContextBreakdown {
    pub system_plugins_skills: u64, // system prompts, CLAUDE.md, plugins, skills
    pub user_messages: u64,         // actual user text input
    pub tool_results: u64,          // tool call responses
    pub assistant_output: u64,      // AI responses
    pub images: u64,                // image tokens (estimated)
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub file_path: String,
    pub title: String,
    pub cwd: String,
    pub git_branch: String,
    pub model: String,               // most recent PRIMARY model (ignoring subagents)
    pub models_timeline: Vec<(usize, String)>,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub turns: usize,
    pub user_turns: usize,
    pub tool_calls: usize,
    pub total_input: u64,
    pub total_output: u64,
    pub total_cache_read: u64,
    pub total_cache_write: u64,
    pub web_searches: u32,
    pub last_context_read: u64,       // last cache_read = approximate context size
    pub last_cache_write: u64,        // last cache_creation
    pub mcp_tools: HashMap<String, u32>,
    pub effort_changes: Vec<(String, String)>,
    pub messages: Vec<MessageInfo>,
    pub context_breakdown: ContextBreakdown,
    pub parent_session_id: Option<String>, // for agent subagents
    pub compressions: Vec<usize>,            // turn indices where context was compressed
    pub waiting_state: WaitingState,
    pub last_file_size: u64,                 // bytes at last full parse
}

pub fn context_window_for_model(model: &str) -> u64 {
    if model.contains("opus") {
        1_000_000
    } else {
        200_000
    }
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    ToolUse { name: String, summary: String, old_str: String, new_str: String },
    ToolResult(String),
    Thinking,
}

#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub role: String,
    pub block: ContentBlock,
}

pub struct SessionStore {
    pub sessions: Vec<Session>,
    pub current_session_id: Option<String>,
    pub current_effort: String,
    #[allow(dead_code)]
    pub stats_cache: Option<StatsCache>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StatsCache {
    pub total_sessions: u64,
    pub total_messages: u64,
    pub weekly_opus_tokens: u64,
    pub weekly_sonnet_tokens: u64,
    pub plan: String,
}

const MODEL_SHORT: &[(&str, &str)] = &[
    ("claude-opus-4-6", "Opus 4.6"),
    ("claude-opus-4-5-20251101", "Opus 4.5"),
    ("claude-sonnet-4-6", "Sonnet 4.6"),
    ("claude-sonnet-4-5-20250929", "Sonnet 4.5"),
    ("claude-haiku-4-5-20251001", "Haiku 4.5"),
];

pub fn short_model(model: &str) -> String {
    for (full, short) in MODEL_SHORT {
        if model == *full {
            return short.to_string();
        }
    }
    if model.len() > 15 {
        model[..15].to_string()
    } else {
        model.to_string()
    }
}

/// Parse raw MCP server key into a friendly display name.
/// e.g. "plugin_github_github" → "GitHub", "claude_ai_Notion" → "Notion"
pub fn friendly_mcp_name(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("claude_ai_") {
        return rest.to_string();
    }
    if let Some(rest) = raw.strip_prefix("plugin_") {
        // plugin_github_github → take first segment after "plugin_"
        if let Some(name) = rest.split('_').next() {
            let mut chars = name.chars();
            if let Some(first) = chars.next() {
                return first.to_uppercase().to_string() + chars.as_str();
            }
        }
    }
    // Bare name like "perplexity" → capitalize
    let mut chars = raw.chars();
    if let Some(first) = chars.next() {
        first.to_uppercase().to_string() + chars.as_str()
    } else {
        raw.to_string()
    }
}

/// Sanitize text for terminal display:
/// - Replace tabs with spaces (tabs cause cursor misalignment in TUI)
/// - Strip ANSI escape sequences (ESC[...m color codes etc.)
/// - Remove other control characters (except newline)
fn sanitize_for_display(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip ANSI escape sequence: ESC[ ... <letter>
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() || c == '~' {
                        break;
                    }
                }
            }
            // Skip other ESC sequences (ESC + one char)
            else if chars.peek().is_some() {
                chars.next();
            }
        } else if ch == '\t' {
            result.push_str("  ");
        } else if ch == '\n' {
            result.push(ch);
        } else if ch.is_control() {
            // Skip other control characters
        } else {
            result.push(ch);
        }
    }
    result
}

/// Strip XML-like tags from preview text (system reminders, command tags, etc.)
fn strip_xml_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' && chars.peek().map(|c| c.is_alphabetic() || *c == '/').unwrap_or(false) {
            in_tag = true;
        } else if ch == '>' && in_tag {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
}

fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    // Handle "2026-03-27T01:09:53.977Z"
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            // Try without timezone
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
                .ok()
                .map(|ndt| ndt.and_utc())
        })
}

pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn fmt_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{}s", secs as u64)
    } else if secs < 3600.0 {
        format!("{}m {}s", secs as u64 / 60, secs as u64 % 60)
    } else {
        let h = secs as u64 / 3600;
        let m = (secs as u64 % 3600) / 60;
        format!("{}h {}m", h, m)
    }
}

pub fn fmt_ago(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(*dt);
    let secs = delta.num_seconds();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", delta.num_days())
    }
}

/// Live MCP connection status from `claude mcp list`.
#[derive(Debug, Clone)]
pub enum McpConnectionStatus {
    Connected,
    NeedsAuth,
    Failed,
}

#[derive(Debug, Clone)]
pub struct McpStatus {
    pub display_name: String,
    pub status: McpConnectionStatus,
}

/// Parse the output of `claude mcp list` into McpStatus entries.
///
/// Each data line looks like:
///   `claude.ai Linear: https://mcp.linear.app/mcp - ✓ Connected`
///   `plugin:github:github: https://... (HTTP) - ! Needs authentication`
///   `plugin:telegram:telegram: bun run ... - ✗ Failed to connect`
pub fn parse_mcp_list_output(output: &str) -> Vec<McpStatus> {
    let mut statuses = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Checking") {
            continue;
        }
        // Status is after the last " - "
        let Some(dash_pos) = line.rfind(" - ") else { continue };
        let status_text = &line[dash_pos + 3..];

        let status = if status_text.contains("Connected") {
            McpConnectionStatus::Connected
        } else if status_text.contains("Needs authentication") {
            McpConnectionStatus::NeedsAuth
        } else {
            McpConnectionStatus::Failed
        };

        // Name: everything before ": <url/command>" — find ": " followed by non-colon
        // Format examples:
        //   "claude.ai Linear: https://mcp.linear.app/mcp - ✓ Connected"
        //   "plugin:github:github: https://api.githubcopilot.com/mcp/ (HTTP) - ✓ Connected"
        //   "perplexity: npx -y @perplexity-ai/mcp-server - ✓ Connected"
        let before_status = &line[..dash_pos].trim_end();
        // Find the last ": " that separates name from URL/command
        let raw_name = if let Some(sep) = before_status.rfind(": ") {
            &before_status[..sep]
        } else {
            before_status
        };

        // Clean up display name
        let display_name = if let Some(rest) = raw_name.strip_prefix("plugin:") {
            // "plugin:github:github" → take first segment after "plugin:"
            let name = rest.split(':').next().unwrap_or(rest);
            let mut c = name.chars();
            match c.next() {
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
                None => name.to_string(),
            }
        } else {
            raw_name.to_string()
        };

        statuses.push(McpStatus { display_name, status });
    }
    statuses
}


/// Extract a human-readable name from a named agent ID.
/// Named agents have filenames like "agent-<name>-<hex_id>.jsonl".
/// Returns None for unnamed agents ("agent-<hex_id>").
fn extract_agent_name(id: &str) -> Option<String> {
    let rest = id.strip_prefix("agent-")?;
    // Split on last '-' to separate potential name from hex session id
    let last_dash = rest.rfind('-')?;
    let potential_name = &rest[..last_dash];
    let potential_hex = &rest[last_dash + 1..];
    // Hex part must be all hex digits; name must contain at least one non-hex char
    let is_hex = !potential_hex.is_empty() && potential_hex.chars().all(|c| c.is_ascii_hexdigit());
    let has_non_hex = potential_name.chars().any(|c| !c.is_ascii_hexdigit());
    if is_hex && has_non_hex {
        // Convert underscores/hyphens to spaces, title-case each word
        let name = potential_name.replace(['_', '-'], " ");
        let titled = name.split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Some(titled)
    } else {
        None
    }
}

fn load_session_from_file(path: &Path, parent_id: Option<String>) -> Option<Session> {
    let content = fs::read_to_string(path).ok()?;
    let id = path.file_stem()?.to_str()?.to_string();

    let mut session = Session {
        id,
        file_path: path.to_string_lossy().to_string(),
        title: String::new(),
        cwd: String::new(),
        git_branch: String::new(),
        model: String::new(),
        models_timeline: Vec::new(),
        start_ts: None,
        end_ts: None,
        turns: 0,
        user_turns: 0,
        tool_calls: 0,
        total_input: 0,
        total_output: 0,
        total_cache_read: 0,
        total_cache_write: 0,
        web_searches: 0,
        last_context_read: 0,
        last_cache_write: 0,
        mcp_tools: HashMap::new(),
        effort_changes: Vec::new(),
        messages: Vec::new(),
        parent_session_id: parent_id,
        compressions: Vec::new(),
        waiting_state: WaitingState::None,
        last_file_size: 0,
        context_breakdown: ContextBreakdown {
            system_plugins_skills: 0,
            user_messages: 0,
            tool_results: 0,
            assistant_output: 0,
            images: 0,
        },
    };

    let mut waiting_state = WaitingState::None;

    for line in content.lines() {
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Custom title
        if entry_type == "custom-title" {
            if let Some(t) = entry.get("customTitle").and_then(|v| v.as_str()) {
                session.title = t.to_string();
            }
        }

        // Effort changes from /effort commands
        if entry_type == "system" {
            if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
                if content.contains("/effort") {
                    // Try to extract effort level
                    if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                        for level in &["low", "medium", "high", "max"] {
                            if content.contains(level) {
                                session.effort_changes.push((ts.to_string(), level.to_string()));
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Timestamps and cwd from user/assistant entries
        if entry_type == "user" || entry_type == "assistant" {
            if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                if let Some(ts) = parse_ts(ts_str) {
                    if session.start_ts.is_none() {
                        session.start_ts = Some(ts);
                    }
                    session.end_ts = Some(ts);
                }
            }
            if session.cwd.is_empty() {
                if let Some(cwd) = entry.get("cwd").and_then(|v| v.as_str()) {
                    session.cwd = cwd.to_string();
                }
            }
            if let Some(branch) = entry.get("gitBranch").and_then(|v| v.as_str()) {
                session.git_branch = branch.to_string();
            }
        }

        // System messages → context breakdown
        if entry_type == "system" {
            if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
                // Rough token estimate: ~4 chars per token
                session.context_breakdown.system_plugins_skills += (content.len() as u64) / 4;
            }
        }

        // User messages — emit text blocks and tool_result blocks
        if entry_type == "user" {
            session.user_turns += 1;
            if let Some(msg) = entry.get("message") {
                if let Some(content) = msg.get("content") {
                    if let Some(arr) = content.as_array() {
                        for block in arr {
                            let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match btype {
                                "text" => {
                                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                        let size = (text.len() as u64) / 4;
                                        if text.contains("<system-reminder>") || text.contains("CLAUDE.md")
                                            || text.contains("<command-name>")
                                        {
                                            session.context_breakdown.system_plugins_skills += size;
                                        } else {
                                            session.context_breakdown.user_messages += size;
                                            let cleaned = sanitize_for_display(&strip_xml_tags(&text.chars().take(2000).collect::<String>()));
                                            let trimmed = cleaned.trim();
                                            if !trimmed.is_empty() && !trimmed.contains("Caveat: The messages below") {
                                                waiting_state = WaitingState::None; // real user text
                                                session.messages.push(MessageInfo {
                                                    role: "user".into(),
                                                    block: ContentBlock::Text(trimmed.to_string()),
                                                });
                                            }
                                        }
                                    }
                                }
                                "tool_result" => {
                                    if waiting_state == WaitingState::WaitingForPermission {
                                        waiting_state = WaitingState::None;
                                    }
                                    let size = block.to_string().len() as u64 / 4;
                                    session.context_breakdown.tool_results += size;
                                    // Extract tool result content
                                    let raw_text = block.get("content")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .chars().take(500).collect::<String>();
                                    let result_text = sanitize_for_display(&raw_text);
                                    if !result_text.trim().is_empty() {
                                        session.messages.push(MessageInfo {
                                            role: "user".into(),
                                            block: ContentBlock::ToolResult(result_text),
                                        });
                                    }
                                }
                                "image" => {
                                    session.context_breakdown.images += 1600;
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(s) = content.as_str() {
                        let cleaned = sanitize_for_display(&strip_xml_tags(&s.chars().take(2000).collect::<String>()));
                        let trimmed = cleaned.trim();
                        if !trimmed.is_empty() {
                            waiting_state = WaitingState::None; // real user text
                            let size = (s.len() as u64) / 4;
                            session.context_breakdown.user_messages += size;
                            session.messages.push(MessageInfo {
                                role: "user".into(),
                                block: ContentBlock::Text(trimmed.to_string()),
                            });
                        }
                    }
                }
            }
        }

        // Assistant messages — emit one ContentBlock per content item
        if entry_type == "assistant" {
            if let Some(msg) = entry.get("message") {
                // Model tracking
                if let Some(m) = msg.get("model").and_then(|v| v.as_str()) {
                    if m != "<synthetic>" {
                        if m != session.model {
                            session.models_timeline.push((session.turns + 1, m.to_string()));
                        }
                        session.model = m.to_string();
                    }
                }

                // Waiting state from stop_reason
                match msg.get("stop_reason").and_then(|v| v.as_str()) {
                    Some("end_turn") => waiting_state = WaitingState::WaitingForInput,
                    Some("tool_use") => waiting_state = WaitingState::WaitingForPermission,
                    _ => {}
                }

                // Usage
                if let Some(usage) = msg.get("usage") {
                    let inp = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let out = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cr = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cw = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let ws = usage.get("server_tool_use")
                        .and_then(|v| v.get("web_search_requests"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    session.total_input += inp;
                    session.total_output += out;
                    session.total_cache_read += cr;
                    session.total_cache_write += cw;
                    session.web_searches += ws as u32;
                    if cr > 0 {
                        if session.last_context_read > 50000 && cr < session.last_context_read * 2 / 5 {
                            session.compressions.push(session.messages.len());
                        }
                        session.last_context_read = cr;
                    }
                    if cw > 0 { session.last_cache_write = cw; }
                    session.context_breakdown.assistant_output += out;

                    // Parse each content block
                    if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            match block.get("type").and_then(|v| v.as_str()) {
                                Some("tool_use") => {
                                    session.tool_calls += 1;
                                    session.turns += 1;
                                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                    if name.starts_with("mcp__") {
                                        let parts: Vec<&str> = name.split("__").collect();
                                        if parts.len() > 1 {
                                            *session.mcp_tools.entry(parts[1].to_string()).or_insert(0) += 1;
                                        }
                                    }
                                    let input = block.get("input");
                                    let summary = sanitize_for_display(&input.and_then(|v| {
                                        v.get("command").and_then(|c| c.as_str()).map(|s| s.chars().take(80).collect::<String>())
                                        .or_else(|| v.get("file_path").and_then(|c| c.as_str()).map(|s| s.to_string()))
                                        .or_else(|| v.get("pattern").and_then(|c| c.as_str()).map(|s| s.to_string()))
                                    }).unwrap_or_default());
                                    let old_str = sanitize_for_display(input.and_then(|v| v.get("old_string").and_then(|s| s.as_str()))
                                        .unwrap_or(""));
                                    let new_str = sanitize_for_display(input.and_then(|v| v.get("new_string").and_then(|s| s.as_str()))
                                        .unwrap_or(""));
                                    session.messages.push(MessageInfo {
                                        role: "assistant".into(),
                                        block: ContentBlock::ToolUse {
                                            name: name.to_string(),
                                            summary,
                                            old_str,
                                            new_str,
                                        },
                                    });
                                }
                                Some("text") => {
                                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                        let cleaned = sanitize_for_display(&strip_xml_tags(&text.chars().take(2000).collect::<String>()));
                                        let trimmed = cleaned.trim();
                                        if !trimmed.is_empty() {
                                            session.turns += 1;
                                            session.messages.push(MessageInfo {
                                                role: "assistant".into(),
                                                block: ContentBlock::Text(trimmed.to_string()),
                                            });
                                        }
                                    }
                                }
                                Some("thinking") => {
                                    session.messages.push(MessageInfo {
                                        role: "assistant".into(),
                                        block: ContentBlock::Thinking,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    session.waiting_state = waiting_state;

    if session.turns == 0 {
        return None;
    }

    // Default title
    if session.title.is_empty() {
        if let Some(name) = extract_agent_name(&session.id) {
            session.title = name;
        } else {
            session.title = session.id.chars().take(14).collect();
        }
    }

    // Clean cwd
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy().to_string();
        session.cwd = session.cwd.replace(&home_str, "~");
    }

    session.last_file_size = content.len() as u64;

    Some(session)
}

impl SessionStore {
    pub fn load() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let projects_dir = home.join(".claude").join("projects");
        let mut sessions: Vec<Session> = Vec::new();
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Collect all JSONL files with modification times
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        if projects_dir.exists() {
            Self::collect_jsonl_files(&projects_dir, &mut files);
        }

        // Sort by modification time, most recent first
        files.sort_by(|a, b| b.1.cmp(&a.1));

        // Load max 40 most recent sessions
        for (path, _) in files {
            if sessions.len() >= 40 {
                break;
            }
            let id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if seen_ids.contains(&id) {
                continue;
            }
            seen_ids.insert(id.clone());

            // Detect parent session for agents: .../parent-id/subagents/agent-*.jsonl
            let parent_id = if id.starts_with("agent-") {
                path.parent() // subagents/
                    .and_then(|p| p.parent()) // parent-session-id/
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            if let Some(session) = load_session_from_file(&path, parent_id) {
                sessions.push(session);
            }
        }

        // Live session: most recent end_ts within last 10 minutes
        let current_session_id = Self::find_live_session(&sessions);

        // Current effort
        let current_effort = Self::read_effort(&home);

        // Stats cache
        let stats_cache = Self::load_stats_cache(&home);

        SessionStore {
            sessions,
            current_session_id,
            current_effort,
            stats_cache,
        }
    }

    /// Fast incremental refresh: only read new bytes from each session file
    /// to update waiting_state and turns without re-parsing everything.
    pub fn refresh_waiting_states(&mut self) {
        use std::io::{Read, Seek, SeekFrom};

        for session in &mut self.sessions {
            let file_size = fs::metadata(&session.file_path)
                .map(|m| m.len())
                .unwrap_or(0);

            // Skip if file hasn't grown
            if file_size <= session.last_file_size {
                continue;
            }

            // Read only new bytes
            let mut file = match fs::File::open(&session.file_path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if file.seek(SeekFrom::Start(session.last_file_size)).is_err() {
                continue;
            }
            let mut new_bytes = Vec::new();
            if file.read_to_end(&mut new_bytes).is_err() {
                continue;
            }
            let new_content = String::from_utf8_lossy(&new_bytes);

            let mut waiting = session.waiting_state.clone();
            let mut turns = session.turns;

            for line in new_content.lines() {
                let entry: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

                if entry_type == "assistant" {
                    if let Some(msg) = entry.get("message") {
                        match msg.get("stop_reason").and_then(|v| v.as_str()) {
                            Some("end_turn") => waiting = WaitingState::WaitingForInput,
                            Some("tool_use") => waiting = WaitingState::WaitingForPermission,
                            _ => {}
                        }
                        // Count new turns from content blocks
                        if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
                            for block in content {
                                match block.get("type").and_then(|v| v.as_str()) {
                                    Some("tool_use") | Some("text") => turns += 1,
                                    _ => {}
                                }
                            }
                        }
                    }
                } else if entry_type == "user" {
                    if let Some(msg) = entry.get("message") {
                        if let Some(content) = msg.get("content") {
                            if let Some(arr) = content.as_array() {
                                for block in arr {
                                    let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                    match btype {
                                        "text" => {
                                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                                if !text.contains("<system-reminder>")
                                                    && !text.contains("CLAUDE.md")
                                                    && !text.contains("<command-name>")
                                                {
                                                    let trimmed = text.trim();
                                                    if !trimmed.is_empty() {
                                                        waiting = WaitingState::None;
                                                    }
                                                }
                                            }
                                        }
                                        "tool_result" => {
                                            if waiting == WaitingState::WaitingForPermission {
                                                waiting = WaitingState::None;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            } else if content.as_str().is_some() {
                                waiting = WaitingState::None;
                            }
                        }
                    }
                }
            }

            session.waiting_state = waiting;
            session.turns = turns;
            session.last_file_size = file_size;
        }

        // Also refresh live session detection
        self.current_session_id = Self::find_live_session(&self.sessions);
    }

    fn collect_jsonl_files(dir: &Path, files: &mut Vec<(PathBuf, std::time::SystemTime)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_jsonl_files(&path, files);
                } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Ok(meta) = path.metadata() {
                        if let Ok(mtime) = meta.modified() {
                            files.push((path, mtime));
                        }
                    }
                }
            }
        }
    }

    fn find_live_session(sessions: &[Session]) -> Option<String> {
        let now = std::time::SystemTime::now();
        let ten_min = std::time::Duration::from_secs(600);
        let mut best: Option<(&Session, std::time::SystemTime)> = None;

        for s in sessions {
            let mtime = fs::metadata(&s.file_path)
                .and_then(|m| m.modified())
                .ok();
            if let Some(mt) = mtime {
                let age = now.duration_since(mt).unwrap_or(std::time::Duration::MAX);
                if age < ten_min {
                    match best {
                        None => best = Some((s, mt)),
                        Some((_, prev_mt)) if mt > prev_mt => best = Some((s, mt)),
                        _ => {}
                    }
                }
            }
        }

        best.map(|(s, _)| s.id.clone())
    }

    fn load_stats_cache(home: &Path) -> Option<StatsCache> {
        // Read plan
        let plan = fs::read_to_string(home.join(".claude").join("stats-config.json"))
            .ok()
            .and_then(|c| serde_json::from_str::<Value>(&c).ok())
            .and_then(|v| v.get("plan").and_then(|p| p.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".into());

        // Read stats cache
        let content = fs::read_to_string(home.join(".claude").join("stats-cache.json")).ok()?;
        let val: Value = serde_json::from_str(&content).ok()?;

        let total_sessions = val.get("totalSessions").and_then(|v| v.as_u64()).unwrap_or(0);
        let total_messages = val.get("totalMessages").and_then(|v| v.as_u64()).unwrap_or(0);

        // Sum last 7 days of dailyModelTokens
        let mut weekly_opus: u64 = 0;
        let mut weekly_sonnet: u64 = 0;

        if let Some(daily) = val.get("dailyModelTokens").and_then(|v| v.as_array()) {
            let recent = daily.iter().rev().take(7);
            for day in recent {
                if let Some(obj) = day.as_object() {
                    for (model, tokens) in obj {
                        if model == "date" { continue; }
                        let t = tokens.as_u64().unwrap_or(0);
                        if model.contains("opus") {
                            weekly_opus += t;
                        } else if model.contains("sonnet") {
                            weekly_sonnet += t;
                        }
                    }
                }
            }
        }

        Some(StatsCache {
            total_sessions,
            total_messages,
            weekly_opus_tokens: weekly_opus,
            weekly_sonnet_tokens: weekly_sonnet,
            plan,
        })
    }

    fn read_effort(home: &Path) -> String {
        let settings = home.join(".claude").join("settings.json");
        if let Ok(content) = fs::read_to_string(&settings) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(e) = val.get("effortLevel").and_then(|v| v.as_str()) {
                    return e.to_string();
                }
            }
        }
        "medium".to_string()
    }
}
