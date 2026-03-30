use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, CodeBlockKind};
use ratatui::{prelude::*, widgets::*};
use std::time::{Duration, Instant};

use crate::session::{fmt_ago, fmt_duration, fmt_tokens, friendly_mcp_name, short_model, McpConnectionStatus, McpStatus, Session, SessionStore, WaitingState};
use std::sync::{Arc, Mutex};

use crate::session::context_window_for_model;
use crate::terminal::ProcessInfo;

fn archive_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude").join("stats-archive.json")
}

fn load_archive() -> std::collections::HashSet<String> {
    let content = match std::fs::read_to_string(archive_path()) {
        Ok(c) => c,
        Err(_) => return std::collections::HashSet::new(),
    };
    serde_json::from_str::<Vec<String>>(&content)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

pub fn save_archive(ids: &std::collections::HashSet<String>) {
    let vec: Vec<&String> = ids.iter().collect();
    if let Ok(json) = serde_json::to_string_pretty(&vec) {
        let _ = std::fs::write(archive_path(), json);
    }
}

/// Find child agent session IDs for a parent by scanning the filesystem.
/// Agents live at ~/.claude/projects/{project}/{parent_id}/subagents/agent-*.jsonl
pub fn find_child_agent_ids(parent_id: &str) -> Vec<String> {
    let home = dirs::home_dir().unwrap_or_default();
    let projects = home.join(".claude").join("projects");
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&projects) {
        for entry in entries.flatten() {
            let subagents = entry.path().join(parent_id).join("subagents");
            if subagents.is_dir() {
                if let Ok(agents) = std::fs::read_dir(&subagents) {
                    for agent in agents.flatten() {
                        let path = agent.path();
                        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                if stem.starts_with("agent-") {
                                    ids.push(stem.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    ids
}

// Theme colors that work with both dark (navy bg) and light (warm gray bg) iTerm themes
const LABEL: Color = Color::Rgb(140, 140, 170);   // soft lavender-gray for labels
const DIM: Color = Color::Rgb(100, 100, 130);      // dimmer but still readable
const BORDER: Color = Color::Rgb(80, 80, 120);     // visible border
const USER_TEXT: Color = Color::Rgb(160, 155, 180); // "you" rows — light lavender
const PREVIEW: Color = Color::Rgb(130, 128, 155);   // turn preview text
const FOOTER_KEY: Color = Color::Rgb(100, 180, 220); // keybinding hints
const _FOOTER_TXT: Color = Color::Rgb(120, 118, 140); // footer descriptions
const SEL_BG: Color = Color::Rgb(35, 35, 65);       // selected row background

const CLAUDE_COLOR: Color = Color::Rgb(207, 107, 55);

#[derive(Debug, Clone, PartialEq)]
enum MascotState {
    Idle,
    Animating,
}

pub struct Mascot {
    state: MascotState,
    last_scroll: Instant,
    tick: u64,
}

impl Mascot {
    pub fn new() -> Self {
        Self {
            state: MascotState::Idle,
            last_scroll: Instant::now(),
            tick: 0,
        }
    }

    pub fn on_scroll(&mut self) {
        self.state = MascotState::Animating;
        self.last_scroll = Instant::now();
    }

    pub fn update(&mut self) {
        match self.state {
            MascotState::Animating => {
                self.tick += 1;
                if self.last_scroll.elapsed() > Duration::from_millis(150) {
                    self.state = MascotState::Idle;
                }
            }
            MascotState::Idle => {}
        }
    }

    pub fn render(&self, f: &mut Frame, char_area: Rect) {
        // Background-color renderer: each pixel = 2 spaces with bg color
        // 14 columns wide (28 terminal chars), 7 rows (7 terminal lines)

        let tick = match self.state {
            MascotState::Animating => self.tick,
            MascotState::Idle => 0,
        };

        // Eye blink
        let blink = if self.state == MascotState::Idle {
            self.last_scroll.elapsed().as_millis() % 4000 < 200
        } else {
            tick % 40 < 2
        };

        // Wing flap
        let flap = if self.state == MascotState::Animating {
            (tick / 6) % 3
        } else {
            0
        };

        // Resolve dynamic pixels as bools: true = filled, false = empty
        let eye: bool = blink;
        let wl: bool = match flap { 1 => false, 2 => true, _ => true };
        let wr: bool = match flap { 1 => true, 2 => false, _ => true };

        let t = true;
        let o = false;

        let grid: Vec<Vec<bool>> = vec![
            vec![o, o, o, t, t, o, t, t, o, t, t, o, o, o], // bumps
            vec![o, o, t, t, t, t, t, t, t, t, t, t, o, o], // body top
            vec![o, o, t, t, eye, eye, t, t, eye, eye, t, t, o, o], // eyes
            vec![o, o, t, t, t, t, t, t, t, t, t, t, o, o], // body mid
            vec![o, wl, t, t, t, t, t, t, t, t, t, t, wr, o], // arms
            vec![o, o, t, t, t, t, t, t, t, t, t, t, o, o], // body bottom
            vec![o, o, t, o, t, t, o, o, t, t, o, t, o, o], // legs
        ];

        let body_color = CLAUDE_COLOR;
        let filled = Style::default().bg(body_color);
        let mascot_w = 28u16; // 14 cols * 2 chars
        let pad = (char_area.width.saturating_sub(mascot_w) / 2) as usize;

        let mut lines: Vec<Line> = Vec::new();

        // Top padding if space allows
        let top_pad = char_area.height.saturating_sub(7) / 2;
        for _ in 0..top_pad {
            lines.push(Line::from(""));
        }

        for row in &grid {
            let mut spans: Vec<Span> = vec![Span::raw(" ".repeat(pad))];
            for &px in row {
                if px {
                    spans.push(Span::styled("  ", filled));
                } else {
                    spans.push(Span::raw("  "));
                }
            }
            lines.push(Line::from(spans));
        }

        f.render_widget(Paragraph::new(lines), char_area);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    List,
    Detail,
}

/// A row in the session list — either a real session or an agent summary.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    Session(usize),                          // index into store.sessions
    AgentSummary { parent_id: String, count: usize }, // collapsed "Agents xN" row
}

pub struct App {
    pub store: SessionStore,
    pub mode: AppMode,
    pub cursor: usize,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub display_rows: Vec<DisplayRow>,       // built from filtered_indices with agent rollup
    pub expanded_parents: std::collections::HashSet<String>, // parent IDs with agents expanded
    pub agent_counts: std::collections::HashMap<String, usize>, // parent_id → agent count (rebuilt with display_rows)
    pub list_offset: usize,
    pub tick: u64,
    pub detail_scroll: usize,
    pub chat_fullscreen: bool,
    pub list_info_tab: usize,
    pub list_table_top: u16,                 // Y position of table for mouse clicks
    pub last_click: Option<(std::time::Instant, u16, u16)>, // (time, row, col) for double-click detection
    pub mascot: Mascot,
    pub expanded_msgs: std::collections::HashSet<usize>,
    #[allow(dead_code)]
    pub tool_summary_indices: Vec<usize>,  // msg indices that have tool summaries
    pub chat_area_top: u16,
    pub chat_scroll_y: std::cell::Cell<u16>,
    pub clickable_lines: std::cell::RefCell<Vec<(usize, usize)>>, // (line_index, msg_index) for ToolUse
    matcher: SkimMatcherV2,
    // In-chat search state
    pub chat_search_active: bool,
    pub chat_search_query: String,
    pub chat_search_matches: Vec<usize>,  // line indices with matches
    pub chat_search_current: usize,       // index into chat_search_matches
    pub chat_total_lines: usize,          // total rendered lines (set by draw)
    pub chat_inner_h: usize,              // visible chat height (set by draw)
    pub mouse_captured: bool,             // whether mouse events are captured (vs terminal selection)
    pub chat_max_scroll: usize,           // max valid detail_scroll (set by draw)
    pub seen_sessions: std::collections::HashMap<String, usize>, // session_id → turns at time of dismissal
    pub archived_ids: std::collections::HashSet<String>,  // session IDs hidden from main list
    pub viewing_archive: bool,                             // true = showing archived sessions only
    pub selected_ids: std::collections::HashSet<String>,   // multi-selected session IDs
    pub status_message: Option<(String, std::time::Instant)>, // transient footer message
    pub process_map: std::collections::HashMap<String, ProcessInfo>, // session_id → running process info
    pub our_tty: Option<String>, // TTY of this claude-stats process (for self-detection)
    pub mcp_statuses: Vec<McpStatus>,                                  // live MCP connection statuses
    pub mcp_loading: bool,
    pub mcp_result: Arc<Mutex<Option<Vec<McpStatus>>>>,
    pub mcp_cursor: usize,                                              // selected MCP row (when on MCPs tab)
}

impl App {
    pub fn new(store: SessionStore) -> Self {
        let archived = load_archive();
        // Pre-filter: exclude archived sessions and agents of archived parents
        let filtered_indices: Vec<usize> = (0..store.sessions.len())
            .filter(|i| {
                let s = &store.sessions[*i];
                let parent_archived = s.parent_session_id.as_ref()
                    .map(|pid| archived.contains(pid))
                    .unwrap_or(false);
                !archived.contains(&s.id) && !parent_archived
            })
            .collect();
        App {
            store,
            mode: AppMode::List,
            cursor: 0,
            search_query: String::new(),
            filtered_indices,
            display_rows: Vec::new(),
            expanded_parents: std::collections::HashSet::new(),
            agent_counts: std::collections::HashMap::new(),
            list_offset: 0,
            tick: 0,
            detail_scroll: 0,
            chat_fullscreen: false,
            list_info_tab: 0,
            list_table_top: 0,
            last_click: None,
            mascot: Mascot::new(),
            expanded_msgs: std::collections::HashSet::new(),
            tool_summary_indices: Vec::new(),
            chat_area_top: 0,
            chat_scroll_y: std::cell::Cell::new(0),
            clickable_lines: std::cell::RefCell::new(Vec::new()),
            matcher: SkimMatcherV2::default(),
            mouse_captured: true,
            chat_search_active: false,
            chat_search_query: String::new(),
            chat_search_matches: Vec::new(),
            chat_search_current: 0,
            chat_total_lines: 0,
            chat_inner_h: 0,
            chat_max_scroll: 0,
            seen_sessions: std::collections::HashMap::new(),
            archived_ids: archived,
            viewing_archive: false,
            selected_ids: std::collections::HashSet::new(),
            status_message: None,
            process_map: std::collections::HashMap::new(),
            our_tty: crate::terminal::current_tty(),
            mcp_statuses: Vec::new(),
            mcp_loading: false,
            mcp_result: Arc::new(Mutex::new(None)),
            mcp_cursor: 0,
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        if self.display_rows.is_empty() {
            return;
        }
        let len = self.display_rows.len();
        if delta < 0 {
            self.cursor = self.cursor.saturating_sub((-delta) as usize);
        } else {
            self.cursor = (self.cursor + delta as usize).min(len - 1);
        }
    }

    /// Move cursor by delta, skipping agent rows (used by detail view Left/Right).
    pub fn move_cursor_skip_agents(&mut self, delta: i32) {
        if self.display_rows.is_empty() {
            return;
        }
        let len = self.display_rows.len();
        let mut pos = self.cursor;
        loop {
            if delta < 0 {
                if pos == 0 { break; }
                pos -= 1;
            } else {
                if pos + 1 >= len { break; }
                pos += 1;
            }
            // Skip agent rows (Session rows whose session has a parent_session_id)
            let is_agent = match self.display_rows[pos] {
                DisplayRow::Session(idx) => self.store.sessions[idx].parent_session_id.is_some(),
                DisplayRow::AgentSummary { .. } => true,
            };
            if !is_agent {
                self.cursor = pos;
                break;
            }
        }
    }

    pub fn scroll_to_search_match(&mut self) {
        if let Some(&line_idx) = self.chat_search_matches.get(self.chat_search_current) {
            let max_scroll = self.chat_total_lines.saturating_sub(self.chat_inner_h);
            let half = self.chat_inner_h / 2;
            // scroll_y = max_scroll - detail_scroll (0=bottom, higher=scrolled up)
            // To center line_idx: scroll_y = line_idx.saturating_sub(half)
            let target_scroll_y = line_idx.saturating_sub(half);
            self.detail_scroll = max_scroll.saturating_sub(target_scroll_y).min(max_scroll);
        }
    }

    pub fn update_filtered(&mut self) {
        let archive_filter = |i: &usize| -> bool {
            let s = &self.store.sessions[*i];
            let id = &s.id;
            // Also check if parent is archived — agents follow their parent
            let parent_archived = s.parent_session_id.as_ref()
                .map(|pid| self.archived_ids.contains(pid))
                .unwrap_or(false);
            if self.viewing_archive {
                self.archived_ids.contains(id) || parent_archived
            } else {
                !self.archived_ids.contains(id) && !parent_archived
            }
        };

        if self.search_query.is_empty() {
            // Archive view shows all loaded sessions (no 40-cap — archived sessions beyond
            // the active window are loaded separately and sorted to the end of the vec).
            let max = if self.viewing_archive {
                self.store.sessions.len()
            } else {
                self.store.sessions.len().min(40)
            };
            self.filtered_indices = (0..max)
                .filter(archive_filter)
                .collect();
        } else {
            let query = &self.search_query;
            let mut scored: Vec<(i64, usize)> = self
                .store
                .sessions
                .iter()
                .enumerate()
                .filter(|(i, _)| archive_filter(i))
                .filter_map(|(i, s)| {
                    let haystack = format!("{} {} {}", s.title, s.cwd, short_model(&s.model));
                    self.matcher
                        .fuzzy_match(&haystack, query)
                        .map(|score| (score, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered_indices = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.cursor = 0;
        self.list_offset = 0;
        self.rebuild_display_rows();
    }

    /// Build display_rows from filtered_indices, collapsing agents into summary rows.
    pub fn rebuild_display_rows(&mut self) {
        let mut rows: Vec<DisplayRow> = Vec::new();
        self.agent_counts.clear();

        // First pass: build agent index per parent
        let mut agents_by_parent: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for &idx in &self.filtered_indices {
            let s = &self.store.sessions[idx];
            if let Some(ref pid) = s.parent_session_id {
                *self.agent_counts.entry(pid.clone()).or_insert(0) += 1;
                agents_by_parent.entry(pid.clone()).or_default().push(idx);
            }
        }

        // Second pass: emit parents with their agents immediately after; skip standalone agent rows
        for &idx in &self.filtered_indices {
            let s = &self.store.sessions[idx];
            if s.parent_session_id.is_some() {
                continue; // emitted below when parent is encountered
            }
            rows.push(DisplayRow::Session(idx));
            if let Some(agent_idxs) = agents_by_parent.get(&s.id) {
                if self.expanded_parents.contains(&s.id) {
                    for &aidx in agent_idxs {
                        rows.push(DisplayRow::Session(aidx));
                    }
                } else {
                    rows.push(DisplayRow::AgentSummary {
                        parent_id: s.id.clone(),
                        count: agent_idxs.len(),
                    });
                }
            }
        }

        self.display_rows = rows;
    }

    pub fn selected_session(&self) -> Option<&Session> {
        match self.display_rows.get(self.cursor) {
            Some(DisplayRow::Session(idx)) => self.store.sessions.get(*idx),
            _ => None,
        }
    }

    /// Returns the waiting indicator string for a session, or None if dismissed/expired/agent.
    pub fn effective_indicator(&self, s: &Session) -> Option<&'static str> {
        if s.parent_session_id.is_some() {
            return None; // never show on agents
        }
        let is_dismissed = self.seen_sessions.get(&s.id)
            .map(|&dismissed_turns| dismissed_turns == s.turns)
            .unwrap_or(false);
        if is_dismissed {
            return None;
        }
        match &s.waiting_state {
            WaitingState::WaitingForInput => {
                let within_hour = s.end_ts
                    .map(|ts| chrono::Utc::now().signed_duration_since(ts).num_seconds() < 3600)
                    .unwrap_or(false);
                if within_hour { Some(" 👋") } else { None }
            }
            WaitingState::WaitingForPermission => Some(" ⏳"),
            WaitingState::None => None,
        }
    }

    fn cleanup_seen_sessions(&mut self) {
        // Remove entries where turns have changed (new activity = indicator can reappear)
        let stale: Vec<String> = self.seen_sessions.iter()
            .filter(|(id, &dismissed_turns)| {
                self.store.sessions.iter()
                    .find(|s| &s.id == *id)
                    .map(|s| s.turns != dismissed_turns)
                    .unwrap_or(true) // session gone = remove
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.seen_sessions.remove(&id);
        }
    }

    /// Fast refresh: only read new bytes from session files to update waiting state.
    pub fn fast_refresh(&mut self) {
        self.store.refresh_waiting_states();
        self.cleanup_seen_sessions();
    }

    pub fn reload_sessions(&mut self) {
        // Remember currently selected session ID to restore position
        let selected_id = self.selected_session().map(|s| s.id.clone());
        let saved_cursor = self.cursor;
        let saved_offset = self.list_offset;

        self.store = SessionStore::load();
        self.cleanup_seen_sessions();

        self.update_filtered();

        // Restore cursor to same session (or same position if session ID not found)
        if let Some(id) = selected_id {
            let mut found = false;
            for (i, row) in self.display_rows.iter().enumerate() {
                if let DisplayRow::Session(idx) = row {
                    if self.store.sessions.get(*idx).map(|s| &s.id) == Some(&id) {
                        self.cursor = i;
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                // Session not in new list — keep cursor at same position, clamped
                self.cursor = saved_cursor.min(self.display_rows.len().saturating_sub(1));
            }
        } else {
            self.cursor = saved_cursor.min(self.display_rows.len().saturating_sub(1));
        }
        self.list_offset = saved_offset;
    }
}

/// Trigger a background MCP health check if not already running.
pub fn trigger_mcp_check(app: &mut App) {
    if app.mcp_loading { return; }
    app.mcp_loading = true;
    app.mcp_statuses.clear();
    let result = Arc::clone(&app.mcp_result);
    std::thread::spawn(move || {
        let output = std::process::Command::new("claude")
            .args(["mcp", "list"])
            .output();
        let statuses = match output {
            Ok(o) => crate::session::parse_mcp_list_output(
                &String::from_utf8_lossy(&o.stdout),
            ),
            Err(_) => Vec::new(),
        };
        *result.lock().unwrap() = Some(statuses);
    });
}

/// Poll for MCP check completion (non-blocking).
pub fn poll_mcp_result(app: &mut App) {
    if !app.mcp_loading { return; }
    if let Ok(mut guard) = app.mcp_result.try_lock() {
        if let Some(statuses) = guard.take() {
            app.mcp_statuses = statuses;
            app.mcp_loading = false;
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.mode {
        AppMode::List => draw_list(f, app),
        AppMode::Detail => draw_detail(f, app),
    }
}

/// Render the MCPs panel with live connection statuses.
fn render_mcp_panel(f: &mut Frame, app: &App, tab_spans: Vec<Span<'_>>, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Tab bar line with summary
    let mut header = tab_spans;
    if app.mcp_loading {
        header.push(Span::styled("Checking MCP connections…", Style::default().fg(Color::Yellow)));
    } else if app.mcp_statuses.is_empty() {
        header.push(Span::styled("Press R to check", Style::default().fg(DIM)));
    } else {
        let connected = app.mcp_statuses.iter()
            .filter(|m| matches!(m.status, McpConnectionStatus::Connected))
            .count();
        let total = app.mcp_statuses.len();
        let issues = total - connected;
        let color = if issues > 0 { Color::Yellow } else { Color::Green };
        header.push(Span::styled(
            format!("{}/{} connected", connected, total),
            Style::default().fg(color).bold(),
        ));
    }
    lines.push(Line::from(header));

    if app.mcp_loading {
        // Pulsing dots based on tick
        let dots = ".".repeat(((app.tick / 5) % 4) as usize);
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<4}", dots), Style::default().fg(Color::Yellow)),
        ]));
    } else {
        for (i, mcp) in app.mcp_statuses.iter().enumerate() {
            let selected = i == app.mcp_cursor;
            let (icon, icon_color, status_text, status_color) = match &mcp.status {
                McpConnectionStatus::Connected => ("✓", Color::Green, "Connected", Color::Green),
                McpConnectionStatus::NeedsAuth => ("!", Color::Yellow, "Needs Auth", Color::Yellow),
                McpConnectionStatus::Failed => ("✗", Color::Red, "Failed", Color::Red),
            };
            let name_trunc = if mcp.display_name.len() > 24 {
                format!("{}…", &mcp.display_name[..23])
            } else {
                mcp.display_name.clone()
            };
            let is_actionable = !matches!(mcp.status, McpConnectionStatus::Connected);
            let cursor_indicator = if selected { "▸" } else { " " };
            let name_style = if selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::White)
            };
            let mut spans = vec![
                Span::styled(cursor_indicator, Style::default().fg(Color::Cyan).bold()),
                Span::styled(" ", Style::default()),
                Span::styled(icon, Style::default().fg(icon_color).bold()),
                Span::styled(" ", Style::default()),
                Span::styled(format!("{:<25}", name_trunc), name_style),
                Span::styled(status_text, Style::default().fg(status_color)),
            ];
            if selected && is_actionable {
                spans.push(Span::styled("  Enter to re-auth", Style::default().fg(DIM)));
            }
            lines.push(Line::from(spans));
        }
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_list(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Clear, area);

    let info_height: u16 = if app.list_info_tab == 4 {
        (app.mcp_statuses.len() as u16 + 2).clamp(4, 14)
    } else {
        2
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // header
            Constraint::Length(2),            // search
            Constraint::Min(10),              // table
            Constraint::Length(info_height),  // info bar
            Constraint::Length(1),            // footer
        ])
        .split(area);

    // Header
    let header_title = if app.viewing_archive { "  Archive " } else { "  Claude Stats " };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(header_title, Style::default().bold().fg(Color::White)),
        Span::styled(
            format!("  {} sessions", app.display_rows.iter().filter(|r| matches!(r, DisplayRow::Session(_))).count()),
            Style::default().fg(LABEL),
        ),
    ]))
    .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(LABEL)));
    f.render_widget(header, chunks[0]);

    // Search bar
    let search_text = if app.search_query.is_empty() {
        Line::from(vec![
            Span::styled("  Search: ", Style::default().fg(LABEL)),
            Span::styled("type to filter...", Style::default().fg(DIM)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  Search: ", Style::default().fg(LABEL)),
            Span::styled(&app.search_query, Style::default().bold().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Cyan)),
        ])
    };
    f.render_widget(Paragraph::new(search_text), chunks[1]);

    // Session table
    app.list_table_top = chunks[2].y;
    let visible_height = chunks[2].height.saturating_sub(3) as usize; // header + borders
    // Clamp cursor to display_rows length
    if !app.display_rows.is_empty() && app.cursor >= app.display_rows.len() {
        app.cursor = app.display_rows.len() - 1;
    }
    if app.cursor >= app.list_offset + visible_height {
        app.list_offset = app.cursor + 1 - visible_height;
    }
    if app.cursor < app.list_offset {
        app.list_offset = app.cursor;
    }

    let current_sid = app.store.current_session_id.as_deref();

    let header_cells = [
        "", "Title", "Model", "Effort", "Tokens", "Turns", "MCPs",
        "When", "Duration",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(LABEL).bold()))
    .collect::<Vec<_>>();

    let header_row = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .display_rows
        .iter()
        .enumerate()
        .skip(app.list_offset)
        .take(visible_height)
        .map(|(i, row)| {
            // Handle agent summary rows (collapsed agents)
            if let DisplayRow::AgentSummary { parent_id: _, count } = row {
                let is_selected = i == app.cursor;
                let summary_style = if is_selected {
                    Style::default().bg(SEL_BG)
                } else {
                    Style::default()
                };
                let marker = if is_selected { "▶ " } else { "  " };
                return Row::new(vec![
                    Cell::from(marker).style(if is_selected { Style::default().fg(FOOTER_KEY).bold() } else { Style::default() }),
                    Cell::from(format!("    ⤷ {} agents", count)).style(Style::default().fg(DIM)),
                    Cell::from(""), Cell::from(""), Cell::from(""),
                    Cell::from(""), Cell::from(""), Cell::from(""), Cell::from(""),
                ]).style(summary_style);
            }

            let idx = match row {
                DisplayRow::Session(idx) => *idx,
                _ => unreachable!(),
            };
            let s = &app.store.sessions[idx];
            let is_selected = i == app.cursor;
            let is_live = current_sid.map(|c| c == s.id).unwrap_or(false);

            let is_agent = s.parent_session_id.is_some();
            let process_info = if !is_agent { app.process_map.get(&s.id) } else { None };
            let is_running = process_info.is_some();
            let is_low_confidence = process_info
                .map(|p| p.confidence == crate::terminal::MatchConfidence::Low)
                .unwrap_or(false);
            let is_multi = app.selected_ids.contains(&s.id);
            let marker = if is_selected && is_live {
                "▶●"
            } else if is_selected && is_multi {
                "▶✓"
            } else if is_selected {
                "▶ "
            } else if is_multi {
                " ✓"
            } else if is_live {
                " ●"
            } else if is_running && !is_low_confidence {
                " ◆"
            } else if is_running {
                " ◇"
            } else {
                "  "
            };
            let marker_style = if is_multi && !is_selected {
                Style::default().fg(Color::Rgb(220, 160, 60)).bold()
            } else if is_selected && is_live {
                Style::default().fg(Color::Green).bold()
            } else if is_selected {
                Style::default().fg(FOOTER_KEY).bold()
            } else if is_live {
                Style::default().fg(Color::Green)
            } else if is_running {
                Style::default().fg(Color::Rgb(100, 180, 220))
            } else {
                Style::default()
            };

            let has_agents = app.agent_counts.contains_key(&s.id);
            let agents_expanded = app.expanded_parents.contains(&s.id);
            let raw_title = if is_agent {
                format!("  ⤷ {}", s.title)
            } else if has_agents {
                let arrow = if agents_expanded { "▾" } else { "▸" };
                format!("{} {}", arrow, s.title)
            } else {
                s.title.clone()
            };

            // Determine waiting indicator
            let indicator = app.effective_indicator(s);

            // Don't truncate titles — the table column (Constraint::Min) handles overflow.
            // Hardcoded limits were clipping titles even when the column had room.
            let title = match indicator {
                Some(ind) => format!("{}{}", raw_title, ind),
                None => raw_title,
            };

            let is_permission_waiting = indicator == Some(" ⏳");

            let title_style = if is_permission_waiting {
                // Permission waiting always shows red — even if live or selected
                Style::default().fg(Color::Red).bold()
            } else if is_selected && is_live {
                Style::default().fg(Color::Green).bold()
            } else if is_selected {
                Style::default().fg(Color::White).bold()
            } else if is_live {
                Style::default().fg(Color::Green).bold()
            } else if is_agent {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Yellow)
            };

            let model = short_model(&s.model);

            let effort = if !s.effort_changes.is_empty() {
                s.effort_changes.last().unwrap().1[..3].to_uppercase()
            } else if is_live {
                app.store.current_effort[..3].to_uppercase()
            } else {
                "—".into()
            };

            let dur = match (s.start_ts, s.end_ts) {
                (Some(start), Some(end)) => {
                    fmt_duration((end - start).num_seconds() as f64)
                }
                _ => String::new(),
            };

            let when = s.end_ts.as_ref().map(fmt_ago).unwrap_or_default();

            let mcp_str: String = s
                .mcp_tools
                .keys()
                .take(2)
                .map(|k| friendly_mcp_name(k))
                .collect::<Vec<_>>()
                .join(" ");

            let live_label = if is_live {
                " ◉ live"
            } else {
                ""
            };

            let row_style = if is_selected && is_live {
                // Selected + live: brighter green bg
                Style::default().bg(Color::Rgb(15, 50, 25))
            } else if is_selected {
                Style::default().bg(SEL_BG)
            } else if is_multi {
                Style::default().bg(Color::Rgb(40, 35, 20))
            } else if is_live {
                Style::default().bg(Color::Rgb(10, 38, 18))
            } else {
                Style::default()
            };

            let title_with_label = format!("{}{}", title, live_label);

            Row::new(vec![
                Cell::from(marker).style(marker_style),
                Cell::from(title_with_label).style(title_style),
                Cell::from(model).style(Style::default().fg(
                    if s.model.contains("opus") { Color::Magenta } else { Color::Cyan }
                )),
                Cell::from(effort).style(Style::default().fg(Color::Yellow)),
                Cell::from(fmt_tokens(s.total_input + s.total_output)).style(Style::default().fg(Color::Green)),
                Cell::from(s.turns.to_string()),
                Cell::from(mcp_str).style(Style::default().fg(Color::Blue)),
                Cell::from(when).style(Style::default().fg(LABEL)),
                Cell::from(dur).style(Style::default().fg(LABEL)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),   // marker (▶● when selected+live)
        Constraint::Min(20),     // title + indicators
        Constraint::Length(10),  // model
        Constraint::Length(6),   // effort
        Constraint::Length(8),   // tokens (total)
        Constraint::Length(5),   // turns
        Constraint::Length(12),  // mcps
        Constraint::Length(9),   // when
        Constraint::Length(9),   // duration
    ];

    let table = Table::new(rows, widths)
        .header(header_row)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(if app.viewing_archive { " Archived Sessions " } else { " Sessions " })
                .title_style(Style::default().fg(Color::White).bold()),
        );
    f.render_widget(table, chunks[2]);

    // Info bar — shows details for selected session based on tab
    let selected_idx = match app.display_rows.get(app.cursor) {
        Some(DisplayRow::Session(idx)) => Some(*idx),
        _ => None,
    };
    if let Some(idx) = selected_idx {
        if let Some(s) = app.store.sessions.get(idx) {
            let tab_labels = ["Branch", "Path", "Models", "Archive", "MCPs"];
            let mut tab_spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
            for (i, label) in tab_labels.iter().enumerate() {
                if i == app.list_info_tab {
                    tab_spans.push(Span::styled(format!("[{}]", label), Style::default().fg(Color::White).bold()));
                } else {
                    tab_spans.push(Span::styled(format!(" {} ", label), Style::default().fg(DIM)));
                }
            }
            tab_spans.push(Span::styled("  ", Style::default()));

            if app.list_info_tab == 4 {
                // MCPs tab — live connection status from `claude mcp list`
                render_mcp_panel(f, app, tab_spans, chunks[3]);
            } else {
                let detail_text = match app.list_info_tab {
                    0 => {
                        if s.git_branch.is_empty() {
                            "No branch info".to_string()
                        } else if let Some(name) = s.git_branch.strip_prefix("worktree-") {
                            format!("\u{2387} \u{2294} {}", name)
                        } else {
                            format!("\u{2387} {}", s.git_branch)
                        }
                    }
                    1 => s.cwd.clone(),
                    2 => {
                        if s.models_timeline.is_empty() {
                            short_model(&s.model)
                        } else {
                            s.models_timeline.iter()
                                .map(|(_, m)| short_model(m))
                                .collect::<Vec<_>>()
                                .join(" → ")
                        }
                    }
                    3 => {
                        let count = app.archived_ids.len();
                        if count == 0 {
                            "Archive empty".to_string()
                        } else {
                            format!("{} archived", count)
                        }
                    }
                    _ => String::new(),
                };

                // If it's an agent, show parent info
                let parent_info = if let Some(pid) = &s.parent_session_id {
                    let parent_title = app.store.sessions.iter()
                        .find(|ps| ps.id == *pid)
                        .map(|ps| ps.title.clone())
                        .unwrap_or_else(|| pid[..pid.len().min(12)].to_string());
                    format!("  ⤷ from: {}", parent_title)
                } else {
                    String::new()
                };

                let detail_color = if app.list_info_tab == 0 { Color::Cyan } else { PREVIEW };
                tab_spans.push(Span::styled(detail_text, Style::default().fg(detail_color)));
                if !parent_info.is_empty() {
                    tab_spans.push(Span::styled(parent_info, Style::default().fg(Color::Rgb(140, 120, 180))));
                }

                f.render_widget(Paragraph::new(vec![
                    Line::from(tab_spans),
                ]), chunks[3]);
            }
        }
    } else if app.list_info_tab == 3 || app.list_info_tab == 4 {
        // Empty session list but Archive or MCPs tab selected — still show tab bar
        let tab_labels = ["Branch", "Path", "Models", "Archive", "MCPs"];
        let mut tab_spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
        for (i, label) in tab_labels.iter().enumerate() {
            if i == app.list_info_tab {
                tab_spans.push(Span::styled(format!("[{}]", label), Style::default().fg(Color::White).bold()));
            } else {
                tab_spans.push(Span::styled(format!(" {} ", label), Style::default().fg(DIM)));
            }
        }
        tab_spans.push(Span::styled("  ", Style::default()));
        if app.list_info_tab == 4 {
            render_mcp_panel(f, app, tab_spans, chunks[3]);
        } else {
            let detail = if app.archived_ids.is_empty() { "Archive empty".to_string() } else { format!("{} archived", app.archived_ids.len()) };
            tab_spans.push(Span::styled(detail, Style::default().fg(PREVIEW)));
            f.render_widget(Paragraph::new(vec![Line::from(tab_spans)]), chunks[3]);
        }
    }

    // Footer — context-sensitive based on archive state
    let select_hint = if !app.selected_ids.is_empty() {
        format!("  {} selected", app.selected_ids.len())
    } else {
        String::new()
    };
    let footer = if app.viewing_archive {
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("navigate  ", Style::default().fg(LABEL)),
            Span::styled("S-↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("select  ", Style::default().fg(LABEL)),
            Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("inspect  ", Style::default().fg(LABEL)),
            Span::styled("R ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("unarchive  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("back", Style::default().fg(LABEL)),
            Span::styled(select_hint.clone(), Style::default().fg(Color::Rgb(220, 160, 60)).bold()),
        ]))
    } else if app.list_info_tab == 4 {
        // MCPs tab footer
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("select  ", Style::default().fg(LABEL)),
            Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("re-auth  ", Style::default().fg(LABEL)),
            Span::styled("R ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("refresh  ", Style::default().fg(LABEL)),
            Span::styled("←→ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("tab  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("quit", Style::default().fg(LABEL)),
        ]))
    } else if app.list_info_tab == 3 {
        // Archive tab footer
        Paragraph::new(Line::from(vec![
            Span::styled(" S-↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("select  ", Style::default().fg(LABEL)),
            Span::styled("A ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("archive  ", Style::default().fg(LABEL)),
            Span::styled("V ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("view archive  ", Style::default().fg(LABEL)),
            Span::styled("←→ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("tab  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("quit", Style::default().fg(LABEL)),
            Span::styled(select_hint, Style::default().fg(Color::Rgb(220, 160, 60)).bold()),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("navigate  ", Style::default().fg(LABEL)),
            Span::styled("←→ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("info tab  ", Style::default().fg(LABEL)),
            Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("inspect  ", Style::default().fg(LABEL)),
            Span::styled("type ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("search  ", Style::default().fg(LABEL)),
            Span::styled("K ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("focus  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("quit  ", Style::default().fg(LABEL)),
            Span::styled("C/X ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("clearAll/1  ", Style::default().fg(LABEL)),
            Span::styled("● ", Style::default().fg(Color::Green)),
            Span::styled("active ", Style::default().fg(LABEL)),
            Span::styled("◆ ", Style::default().fg(Color::Rgb(100, 180, 220))),
            Span::styled("running", Style::default().fg(LABEL)),
        ]))
    };
    f.render_widget(footer, chunks[4]);
}

fn draw_detail(f: &mut Frame, app: &mut App) {
    let detail_indicator = app.selected_session().and_then(|s| app.effective_indicator(s));
    let session = match app.selected_session() {
        Some(s) => s.clone(),
        None => return,
    };

    let area = f.area();
    f.render_widget(Clear, area);
    let current_sid = app.store.current_session_id.as_deref();
    let is_live = current_sid.map(|c| c == session.id).unwrap_or(false);

    // Dynamic context window based on model
    let ctx_window = context_window_for_model(&session.model);

    if app.chat_fullscreen {
        // Fullscreen chat — just header + chat + footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header
                Constraint::Min(6),   // chat
                Constraint::Length(1), // footer
            ])
            .split(area);

        // Header
        let ind_suffix = detail_indicator.unwrap_or("");
        let title_text = if is_live {
            format!(" ● {} (live){}", session.title, ind_suffix)
        } else {
            format!(" {}{}", session.title, ind_suffix)
        };
        let title_color = if is_live {
            Color::Green
        } else if detail_indicator == Some(" ⏳") {
            Color::Red
        } else {
            Color::Yellow
        };
        f.render_widget(Paragraph::new(Line::from(
            Span::styled(title_text, Style::default().bold().fg(title_color))
        )), chunks[0]);

        // Chat + mascot
        draw_claude_animation(f, chunks[1], &session, app);

        // Footer — changes based on search state
        let fs_footer = if app.chat_search_active {
            Paragraph::new(Line::from(vec![
                Span::styled(" /", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled(app.chat_search_query.to_string(), Style::default().fg(Color::White)),
                Span::styled("█  ", Style::default().fg(Color::Rgb(255, 160, 40))),
                Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("confirm  ", Style::default().fg(LABEL)),
                Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("cancel", Style::default().fg(LABEL)),
            ]))
        } else if !app.chat_search_query.is_empty() {
            let match_info = if app.chat_search_matches.is_empty() {
                "no matches".to_string()
            } else {
                format!("{}/{}", app.chat_search_current + 1, app.chat_search_matches.len())
            };
            Paragraph::new(Line::from(vec![
                Span::styled(" n", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("ext  ", Style::default().fg(LABEL)),
                Span::styled("N", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("prev  ", Style::default().fg(LABEL)),
                Span::styled("/", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("search  ", Style::default().fg(LABEL)),
                Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("clear  ", Style::default().fg(LABEL)),
                Span::styled(format!("({})", match_info), Style::default().fg(Color::Rgb(255, 160, 40))),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(" f ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("exit fullscreen  ", Style::default().fg(LABEL)),
                Span::styled("↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("scroll  ", Style::default().fg(LABEL)),
                Span::styled("/", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("search  ", Style::default().fg(LABEL)),
                Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("expand/collapse  ", Style::default().fg(LABEL)),
                Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
                Span::styled("back", Style::default().fg(LABEL)),
            ]))
        };
        f.render_widget(fs_footer, chunks[2]);
        return;
    }

    // Responsive: if terminal is short, shrink info panels
    let is_short = area.height < 40;
    let info_h = if is_short { 10 } else { 12 };
    let ctx_h = if is_short { 8 } else { 14 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),      // header (compact)
            Constraint::Length(info_h), // session info + token usage
            Constraint::Length(ctx_h),  // context breakdown
            Constraint::Min(6),        // claude animation + chat
            Constraint::Length(1),     // footer
        ])
        .split(area);

    // ── Header ──
    let ind_suffix = detail_indicator.unwrap_or("");
    let title_text = if is_live {
        format!("  ● {} (live){}", session.title, ind_suffix)
    } else {
        format!("  {}{}", session.title, ind_suffix)
    };
    let title_color = if is_live {
        Color::Green
    } else if detail_indicator == Some(" ⏳") {
        Color::Red
    } else {
        Color::Yellow
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(title_text, Style::default().bold().fg(title_color)),
    ]))
    .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(LABEL)));
    f.render_widget(header, chunks[0]);

    // ── Row 1: Session Info + Token Usage side by side ──
    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Session Info panel
    let model = short_model(&session.model);
    let model_color = if session.model.contains("opus") { Color::Magenta } else { Color::Cyan };
    let dur = match (session.start_ts, session.end_ts) {
        (Some(start), Some(end)) => fmt_duration((end - start).num_seconds() as f64),
        _ => "?".into(),
    };
    let start_str = session.start_ts
        .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".into());

    let effort = if !session.effort_changes.is_empty() {
        session.effort_changes.last().unwrap().1.to_uppercase()
    } else if is_live {
        app.store.current_effort.to_uppercase()
    } else {
        "—".into()
    };

    let ctx_window_str = fmt_tokens(ctx_window);
    let mut info_lines = vec![
        Line::from(vec![
            Span::styled("Model     ", Style::default().fg(LABEL)),
            Span::styled(&model, Style::default().fg(model_color).bold()),
            Span::styled(format!("  ({})", ctx_window_str), Style::default().fg(LABEL)),
        ]),
        Line::from(vec![
            Span::styled("Effort    ", Style::default().fg(LABEL)),
            Span::styled(&effort, Style::default().fg(Color::Yellow).bold()),
        ]),
        Line::from(vec![
            Span::styled("Started   ", Style::default().fg(LABEL)),
            Span::styled(start_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Duration  ", Style::default().fg(LABEL)),
            Span::styled(&dur, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Turns     ", Style::default().fg(LABEL)),
            Span::styled(format!("{} user / {} ai", session.user_turns, session.turns), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Tools     ", Style::default().fg(LABEL)),
            Span::styled(session.tool_calls.to_string(), Style::default().fg(Color::White)),
            if session.web_searches > 0 {
                Span::styled(format!("  web: {}", session.web_searches), Style::default().fg(Color::Cyan))
            } else {
                Span::raw("")
            },
        ]),
    ];

    if session.models_timeline.len() > 1 {
        let switches: String = session.models_timeline.iter()
            .map(|(_, m)| short_model(m))
            .collect::<Vec<_>>()
            .join(" → ");
        info_lines.push(Line::from(vec![
            Span::styled("Switches  ", Style::default().fg(LABEL)),
            Span::styled(switches, Style::default().fg(USER_TEXT)),
        ]));
    }

    if !session.mcp_tools.is_empty() {
        let mcp_str: String = session.mcp_tools.iter()
            .map(|(name, count)| format!("{} ×{}", name, count))
            .collect::<Vec<_>>()
            .join("  ");
        info_lines.push(Line::from(vec![
            Span::styled("MCPs      ", Style::default().fg(LABEL)),
            Span::styled(mcp_str, Style::default().fg(Color::Blue)),
        ]));
    }

    if !session.git_branch.is_empty() {
        let branch_display = if let Some(name) = session.git_branch.strip_prefix("worktree-") {
            format!("\u{2294} {}", name)
        } else {
            session.git_branch.clone()
        };
        info_lines.push(Line::from(vec![
            Span::styled("Branch    ", Style::default().fg(LABEL)),
            Span::styled(branch_display, Style::default().fg(Color::Cyan)),
        ]));
    }

    let info_panel = Paragraph::new(info_lines).block(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(80, 110, 170)))
            .title(" Session ").title_style(Style::default().fg(Color::White).bold()),
    );
    f.render_widget(info_panel, row1[0]);

    // Token Usage panel
    let total_tokens = session.total_input + session.total_output;
    let usage_lines = vec![
        Line::from(vec![
            Span::styled("Output       ", Style::default().fg(LABEL)),
            Span::styled(fmt_tokens(session.total_output), Style::default().fg(Color::Green).bold()),
        ]),
        Line::from(vec![
            Span::styled("Input        ", Style::default().fg(LABEL)),
            Span::styled(fmt_tokens(session.total_input), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Cache Read   ", Style::default().fg(LABEL)),
            Span::styled(fmt_tokens(session.total_cache_read), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Cache Write  ", Style::default().fg(LABEL)),
            Span::styled(fmt_tokens(session.total_cache_write), Style::default().fg(USER_TEXT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Total        ", Style::default().fg(LABEL)),
            Span::styled(fmt_tokens(total_tokens), Style::default().fg(Color::Yellow).bold()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Last turn    ", Style::default().fg(LABEL)),
            Span::styled(format!("read {} + wrote {}", fmt_tokens(session.last_context_read), fmt_tokens(session.last_cache_write)), Style::default().fg(USER_TEXT)),
        ]),
    ];
    let usage_panel = Paragraph::new(usage_lines).block(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(80, 150, 80)))
            .title(" Token Usage ").title_style(Style::default().fg(Color::White).bold()),
    );
    f.render_widget(usage_panel, row1[1]);

    // ── Row 2: Context Breakdown (full width) ──
    let ctx_total = session.last_context_read + session.last_cache_write;
    let ctx_pct = if ctx_total > 0 {
        (ctx_total as f64 / ctx_window as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let bar_color = match ctx_pct as u32 {
        0..=19  => Color::Rgb(100, 140, 200), // soft blue
        20..=39 => Color::Rgb(70, 130, 220),  // blue
        40..=54 => Color::Rgb(80, 180, 120),  // soft green
        55..=69 => Color::Rgb(60, 200, 80),   // green
        70..=79 => Color::Rgb(220, 200, 50),  // yellow
        _       => Color::Rgb(220, 80, 60),   // red (80%+)
    };

    // Context breakdown categories
    let cb = &session.context_breakdown;
    let cb_total = cb.system_plugins_skills + cb.user_messages + cb.tool_results + cb.assistant_output + cb.images;
    let pct = |v: u64| if cb_total > 0 { format!("{:.0}%", v as f64 / cb_total as f64 * 100.0) } else { "—".into() };

    let ctx_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    // Bar width based on the LEFT panel's actual width (not full width)
    let bar_width = (ctx_row[0].width.saturating_sub(4)) as usize;
    let filled = ((ctx_pct / 100.0) * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);

    // Left: context bar and summary
    let ctx_left = vec![
        Line::from(vec![
            Span::styled("Context Window  ", Style::default().fg(LABEL)),
            Span::styled(format!("{} / {}", fmt_tokens(ctx_total), fmt_tokens(ctx_window)), Style::default().fg(Color::White).bold()),
            Span::styled(format!("  ({:.1}%)", ctx_pct), Style::default().fg(bar_color).bold()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
            Span::styled("░".repeat(empty), Style::default().fg(DIM)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{} remaining", fmt_tokens(ctx_window.saturating_sub(ctx_total))), Style::default().fg(USER_TEXT)),
        ]),
    ];

    let ctx_left_panel = Paragraph::new(ctx_left).block(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(160, 160, 70)))
            .title(" Context Usage ").title_style(Style::default().fg(Color::White).bold()),
    );
    f.render_widget(ctx_left_panel, ctx_row[0]);

    // Right: category breakdown (like /context)
    let ctx_right = vec![
        Line::from(vec![
            Span::styled("System/Plugins/Skills  ", Style::default().fg(LABEL)),
            Span::styled(format!("~{}", fmt_tokens(cb.system_plugins_skills)), Style::default().fg(Color::Magenta)),
            Span::styled(format!("  {}", pct(cb.system_plugins_skills)), Style::default().fg(LABEL)),
        ]),
        Line::from(vec![
            Span::styled("User Messages          ", Style::default().fg(LABEL)),
            Span::styled(format!("~{}", fmt_tokens(cb.user_messages)), Style::default().fg(Color::Cyan)),
            Span::styled(format!("  {}", pct(cb.user_messages)), Style::default().fg(LABEL)),
        ]),
        Line::from(vec![
            Span::styled("Tool Results           ", Style::default().fg(LABEL)),
            Span::styled(format!("~{}", fmt_tokens(cb.tool_results)), Style::default().fg(Color::Yellow)),
            Span::styled(format!("  {}", pct(cb.tool_results)), Style::default().fg(LABEL)),
        ]),
        Line::from(vec![
            Span::styled("Assistant Output       ", Style::default().fg(LABEL)),
            Span::styled(format!("~{}", fmt_tokens(cb.assistant_output)), Style::default().fg(Color::Green)),
            Span::styled(format!("  {}", pct(cb.assistant_output)), Style::default().fg(LABEL)),
        ]),
        if cb.images > 0 {
            Line::from(vec![
                Span::styled("Images                 ", Style::default().fg(LABEL)),
                Span::styled(format!("~{}", fmt_tokens(cb.images)), Style::default().fg(Color::Blue)),
                Span::styled(format!("  {}", pct(cb.images)), Style::default().fg(LABEL)),
            ])
        } else {
            Line::from("")
        },
        Line::from(""),
        Line::from(vec![
            Span::styled("Estimated Total        ", Style::default().fg(LABEL)),
            Span::styled(format!("~{}", fmt_tokens(cb_total)), Style::default().fg(Color::White).bold()),
        ]),
    ];

    let ctx_right_panel = Paragraph::new(ctx_right).block(
        Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(160, 160, 70)))
            .title(" Breakdown (estimated) ").title_style(Style::default().fg(Color::White).bold()),
    );
    f.render_widget(ctx_right_panel, ctx_row[1]);

    // ── Row 3: Claude Animation ──
    draw_claude_animation(f, chunks[3], &session, app);

    // Footer — changes based on search state
    let footer = if app.chat_search_active {
        Paragraph::new(Line::from(vec![
            Span::styled(" /", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled(app.chat_search_query.to_string(), Style::default().fg(Color::White)),
            Span::styled("█  ", Style::default().fg(Color::Rgb(255, 160, 40))),
            Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("confirm  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("cancel", Style::default().fg(LABEL)),
        ]))
    } else if !app.chat_search_query.is_empty() {
        let match_info = if app.chat_search_matches.is_empty() {
            "no matches".to_string()
        } else {
            format!("{}/{}", app.chat_search_current + 1, app.chat_search_matches.len())
        };
        Paragraph::new(Line::from(vec![
            Span::styled(" n", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("ext  ", Style::default().fg(LABEL)),
            Span::styled("N", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("prev  ", Style::default().fg(LABEL)),
            Span::styled("/", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("search  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("clear  ", Style::default().fg(LABEL)),
            Span::styled(format!("({})", match_info), Style::default().fg(Color::Rgb(255, 160, 40))),
        ]))
    } else if let Some((ref msg, when)) = app.status_message {
        if when.elapsed().as_secs() < 3 {
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", msg), Style::default().fg(Color::Rgb(100, 200, 140))),
            ]))
        } else {
            app.status_message = None;
            detail_footer_default(app)
        }
    } else {
        detail_footer_default(app)
    };
    f.render_widget(footer, chunks[4]);
}


/// Highlight search matches in rendered lines. Returns indices of lines that contain matches.
fn detail_footer_default(app: &App) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("scroll ", Style::default().fg(LABEL)),
        Span::styled("f", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("ullscreen ", Style::default().fg(LABEL)),
        Span::styled("K", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled(" focus ", Style::default().fg(LABEL)),
        Span::styled("c", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled(" tab ", Style::default().fg(LABEL)),
        Span::styled("C", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled(" here ", Style::default().fg(LABEL)),
        Span::styled("/", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("search ", Style::default().fg(LABEL)),
        Span::styled("m", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled(if app.mouse_captured { "ouse " } else { "ouse:select " }, Style::default().fg(LABEL)),
        Span::styled("←→", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("nav  ", Style::default().fg(LABEL)),
        Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("back", Style::default().fg(LABEL)),
    ]))
}

fn highlight_search_matches(lines: &mut Vec<Line<'_>>, query: &str, current_match_line: Option<usize>) -> Vec<usize> {
    let query_lower = query.to_lowercase();
    let highlight = Style::default().fg(Color::Black).bg(Color::Yellow);
    let current_highlight = Style::default().fg(Color::Black).bg(Color::Rgb(255, 160, 40));
    let mut match_lines: Vec<usize> = Vec::new();

    for (line_idx, line) in lines.iter_mut().enumerate() {
        // Check if any span in this line contains the query
        let has_match = line.spans.iter().any(|s| {
            s.content.to_lowercase().contains(&query_lower)
        });
        if !has_match { continue; }

        match_lines.push(line_idx);
        let is_current = current_match_line == Some(line_idx);
        let hl = if is_current { current_highlight } else { highlight };

        // Split spans at match boundaries
        let old_spans: Vec<Span<'_>> = std::mem::take(&mut line.spans);
        let mut new_spans: Vec<Span<'_>> = Vec::new();

        for span in old_spans {
            let text = span.content.to_string();
            let text_lower = text.to_lowercase();
            let style = span.style;

            if !text_lower.contains(&query_lower) {
                new_spans.push(Span::styled(text, style));
                continue;
            }

            let mut pos = 0;
            let qlen = query_lower.len();

            while pos < text.len() {
                if let Some(found) = text_lower[pos..].find(&query_lower) {
                    let abs = pos + found;
                    if abs > pos {
                        new_spans.push(Span::styled(text[pos..abs].to_string(), style));
                    }
                    new_spans.push(Span::styled(text[abs..abs + qlen].to_string(), hl));
                    pos = abs + qlen;
                } else {
                    new_spans.push(Span::styled(text[pos..].to_string(), style));
                    break;
                }
            }
        }

        *line = Line::from(new_spans);
    }

    match_lines
}

/// Render markdown text to styled ratatui Lines using pulldown-cmark.
/// Handles bold, italic, strikethrough, code spans, code blocks, headings,
/// bullets, numbered lists, blockquotes, links, and word-boundary wrapping.
fn render_markdown_to_lines(
    text: &str,
    text_w: usize,
    badge_spans: Vec<Span<'static>>,
) -> Vec<Line<'static>> {
    let code_color = Color::Rgb(180, 140, 200);
    let heading_color = Color::Rgb(200, 200, 220);
    let bullet_color = Color::Rgb(100, 180, 220);
    let base_color = PREVIEW;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_col: usize = 0;
    let mut first_line = true;

    // Style stack for nested inline formatting
    let mut bold = false;
    let mut italic = false;
    let mut strikethrough = false;

    // Block state
    let mut in_code_block = false;
    let mut in_heading = false;
    let mut in_blockquote = false;
    let mut list_stack: Vec<Option<u64>> = Vec::new(); // None = unordered, Some(n) = ordered at n
    let mut item_started = false;

    let effective_w = if text_w > 2 { text_w } else { 80 };

    // Helper: build the current composite style
    let build_style = |bold: bool, italic: bool, strikethrough: bool, in_heading: bool, in_code: bool| -> Style {
        let mut s = if in_heading {
            Style::default().fg(heading_color).bold()
        } else if in_code {
            Style::default().fg(code_color)
        } else {
            Style::default().fg(base_color)
        };
        if bold { s = s.bold(); }
        if italic { s = s.italic(); }
        if strikethrough { s = s.add_modifier(Modifier::CROSSED_OUT); }
        s
    };

    // Helper: get the prefix spans for the current line
    let make_prefix = |first: &mut bool| -> Vec<Span<'static>> {
        if *first {
            *first = false;
            let mut v = badge_spans.clone();
            v.push(Span::styled(" \u{25b8} ", Style::default().fg(DIM)));
            v
        } else {
            vec![Span::styled("           ".to_string(), Style::default().fg(DIM))]
        }
    };

    // Flush current_spans into a Line
    let flush_line = |lines: &mut Vec<Line<'static>>,
                      current_spans: &mut Vec<Span<'static>>,
                      current_col: &mut usize,
                      first_line: &mut bool| {
        let mut spans = make_prefix(first_line);
        spans.append(current_spans);
        lines.push(Line::from(spans));
        *current_col = 0;
    };

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(text, opts);

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level: _, .. }) => {
                // Flush any pending content before heading
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                in_heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                // Flush the heading line
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                in_heading = false;
            }
            Event::Start(Tag::Emphasis) => { italic = true; }
            Event::End(TagEnd::Emphasis) => { italic = false; }
            Event::Start(Tag::Strong) => { bold = true; }
            Event::End(TagEnd::Strong) => { bold = false; }
            Event::Start(Tag::Strikethrough) => { strikethrough = true; }
            Event::End(TagEnd::Strikethrough) => { strikethrough = false; }

            Event::Start(Tag::CodeBlock(kind)) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                in_code_block = true;
                let label = match &kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => lang.to_string(),
                    _ => "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string(),
                };
                let mut pfx = make_prefix(&mut first_line);
                pfx.push(Span::styled(label, Style::default().fg(code_color).italic()));
                lines.push(Line::from(pfx));
            }
            Event::End(TagEnd::CodeBlock) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                in_code_block = false;
                let mut pfx = make_prefix(&mut first_line);
                pfx.push(Span::styled("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string(), Style::default().fg(code_color).italic()));
                lines.push(Line::from(pfx));
            }

            Event::Start(Tag::BlockQuote(_)) => {
                in_blockquote = true;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_blockquote = false;
            }

            Event::Start(Tag::List(start)) => {
                list_stack.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                item_started = true;
                // Determine list depth indent (2 chars per level beyond first)
                let depth = list_stack.len().saturating_sub(1);
                let indent_str: String = "  ".repeat(depth);
                if !indent_str.is_empty() {
                    current_spans.push(Span::styled(indent_str.clone(), Style::default()));
                    current_col += depth * 2;
                }
                // Emit bullet or number
                if let Some(counter) = list_stack.last().copied().flatten() {
                    let label = format!("{}. ", counter);
                    current_col += label.len();
                    current_spans.push(Span::styled(label, Style::default().fg(bullet_color)));
                    // Increment the counter for next item
                    if let Some(entry) = list_stack.last_mut() {
                        *entry = Some(counter + 1);
                    }
                } else {
                    current_spans.push(Span::styled("\u{2022} ".to_string(), Style::default().fg(bullet_color)));
                    current_col += 2;
                }
            }
            Event::End(TagEnd::Item) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                item_started = false;
            }

            Event::Start(Tag::Paragraph) => {
                // Nothing needed — paragraph start is implicit
            }
            Event::End(TagEnd::Paragraph) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
            }

            Event::Start(Tag::Link { dest_url, .. }) => {
                // We'll render link text normally, then show URL after
                // Link style handled in Text event via bold/italic state
                let _ = dest_url; // URL display handled when we get the text
            }
            Event::End(TagEnd::Link) => {}

            Event::Text(content) => {
                let style = build_style(bold, italic, strikethrough, in_heading, in_code_block);

                if in_code_block {
                    // Code blocks: render each line as-is with code color, no word-wrap
                    for (i, code_line) in content.split('\n').enumerate() {
                        if i > 0 {
                            flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                        }
                        current_spans.push(Span::styled(format!("  {}", code_line), style));
                        current_col += code_line.len() + 2;
                    }
                } else {
                    // Blockquote prefix
                    let bq_prefix = if in_blockquote { "\u{258e} " } else { "" };
                    let bq_cost = if in_blockquote { 2 } else { 0 };
                    let bq_style = Style::default().fg(LABEL);

                    // Word-wrap text content
                    for word in content.split_whitespace() {
                        let word_len = word.chars().count();
                        let need_space = if current_col > 0 && !item_started { 1 } else { 0 };
                        item_started = false;

                        if current_col + need_space + word_len + bq_cost > effective_w && current_col > 0 {
                            // Wrap to next line
                            flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                            if in_blockquote {
                                current_spans.push(Span::styled(bq_prefix.to_string(), bq_style));
                                current_col += bq_cost;
                            }
                        }

                        if current_col == 0 && in_blockquote && current_spans.is_empty() {
                            current_spans.push(Span::styled(bq_prefix.to_string(), bq_style));
                            current_col += bq_cost;
                        }

                        if need_space > 0 && current_col > 0 {
                            current_spans.push(Span::styled(" ".to_string(), style));
                            current_col += 1;
                        }
                        current_spans.push(Span::styled(word.to_string(), style));
                        current_col += word_len;
                    }
                }
            }

            Event::Code(content) => {
                let word = content.to_string();
                let word_len = word.chars().count() + 2; // backtick visual
                let need_space = if current_col > 0 { 1 } else { 0 };

                if current_col + need_space + word_len > effective_w && current_col > 0 {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                if need_space > 0 && current_col > 0 {
                    current_spans.push(Span::styled(" ".to_string(), Style::default().fg(base_color)));
                    current_col += 1;
                }
                current_spans.push(Span::styled(word, Style::default().fg(code_color)));
                current_col += word_len;
            }

            Event::SoftBreak => {
                // Treat as a space (CommonMark default)
                if current_col > 0 {
                    let style = build_style(bold, italic, strikethrough, in_heading, in_code_block);
                    current_spans.push(Span::styled(" ".to_string(), style));
                    current_col += 1;
                }
            }
            Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
            }
            Event::Rule => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
                let mut pfx = make_prefix(&mut first_line);
                let rule_w = effective_w.min(40);
                pfx.push(Span::styled("\u{2500}".repeat(rule_w), Style::default().fg(DIM)));
                lines.push(Line::from(pfx));
            }

            // Table support (basic)
            Event::Start(Tag::Table(_)) | Event::End(TagEnd::Table) => {}
            Event::Start(Tag::TableHead) | Event::End(TagEnd::TableHead) => {}
            Event::Start(Tag::TableRow) | Event::End(TagEnd::TableRow) => {
                if !current_spans.is_empty() {
                    flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
                }
            }
            Event::Start(Tag::TableCell) => {
                if current_col > 0 {
                    current_spans.push(Span::styled(" \u{2502} ".to_string(), Style::default().fg(DIM)));
                    current_col += 3;
                }
            }
            Event::End(TagEnd::TableCell) => {}

            // Catch-all for other events we don't handle
            _ => {}
        }
    }

    // Flush any remaining content
    if !current_spans.is_empty() {
        flush_line(&mut lines, &mut current_spans, &mut current_col, &mut first_line);
    }

    lines
}

fn draw_claude_animation(f: &mut Frame, area: Rect, session: &Session, app: &mut App) {
    let detail_scroll = app.detail_scroll;
    let mascot = &app.mascot;
    let char_h = 9u16;
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(char_h),
        ])
        .split(area);

    let chat_area = parts[0];
    let char_area = parts[1];
    app.chat_area_top = chat_area.y;

    // ── Chat window: preserve formatting, render markdown-like content ──
    app.clickable_lines.borrow_mut().clear();
    let chat_w = chat_area.width.saturating_sub(2) as usize; // inner width minus borders
    let indent_w = 11usize; // badge + " ▸ " prefix width
    let text_w = chat_w.saturating_sub(indent_w); // available width for text content
    let mut lines: Vec<Line> = Vec::new();
    let diff_add = Color::Rgb(80, 200, 80);       // green for +
    let diff_del = Color::Rgb(200, 80, 80);       // red for -

    let compress_color = Color::Rgb(220, 160, 60);
    let tool_color = Color::Rgb(100, 200, 100); // green for tool dots
    let tool_dim = Color::Rgb(100, 100, 130);
    let msgs = &session.messages;

    // Render blocks directly — matching Claude Code's native format
    use crate::session::ContentBlock;

    for (msg_idx, m) in msgs.iter().enumerate() {
        // Compression markers
        if session.compressions.contains(&msg_idx) {
            lines.push(Line::from(vec![
                Span::styled("  ⚡ ", Style::default().fg(compress_color).bold()),
                Span::styled("── Context compressed ──", Style::default().fg(compress_color)),
            ]));
            lines.push(Line::from(""));
        }

        match &m.block {
            ContentBlock::Thinking => {
                // Skip thinking blocks entirely
            }
            ContentBlock::ToolResult(result) => {
                // Show tool result indented under previous tool_use
                let result_prefix_w = 14; // "            └ "
                let max_result_w = text_w.saturating_sub(result_prefix_w);
                for line in result.lines().take(5) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        let truncated: String = trimmed.chars().take(max_result_w).collect();
                        lines.push(Line::from(vec![
                            Span::styled("            └ ", Style::default().fg(tool_dim)),
                            Span::styled(truncated, Style::default().fg(tool_dim)),
                        ]));
                    }
                }
            }
            ContentBlock::ToolUse { name, summary, old_str, new_str } => {
                // Green dot + tool name like Claude Code
                let tool_prefix_w = 12; // "          ● "
                let max_tool_w = text_w.saturating_sub(tool_prefix_w + 2); // reserve for expand arrow
                let full_name = if name == "Edit" || name == "Write" {
                    format!("Update({})", summary)
                } else {
                    format!("{}({})", name, summary)
                };
                let display_name: String = full_name.chars().take(max_tool_w).collect();

                let has_diff = !old_str.is_empty() || !new_str.is_empty();
                let expanded = has_diff && app.expanded_msgs.contains(&msg_idx);

                // Record this line as clickable for expand/collapse
                if has_diff {
                    app.clickable_lines.borrow_mut().push((lines.len(), msg_idx));
                }

                lines.push(Line::from(vec![
                    Span::styled("          ", Style::default()),
                    Span::styled("● ", Style::default().fg(tool_color).bold()),
                    Span::styled(display_name, Style::default().fg(Color::White)),
                    if has_diff {
                        let arrow_color = Color::Rgb(255, 200, 60); // bright yellow arrow
                        let marker = if expanded { " ▼" } else { " ▶" };
                        Span::styled(marker, Style::default().fg(arrow_color).bold())
                    } else {
                        Span::raw("")
                    },
                ]));

                // Show diff if expanded — with line numbers
                if expanded {
                    let del_count = old_str.lines().count();
                    let add_count = new_str.lines().count();
                    let _line_num_w = 4; // width for line numbers
                    lines.push(Line::from(vec![
                        Span::styled("            └ ", Style::default().fg(tool_dim)),
                        Span::styled(format!("Added {} lines, removed {} lines", add_count, del_count), Style::default().fg(tool_dim)),
                    ]));
                    let diff_content_w = text_w.saturating_sub(7); // "NNNN - " prefix
                    let mut line_num = 1usize;
                    for line in old_str.lines() {
                        let truncated: String = line.chars().take(diff_content_w).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!("{:>4} ", line_num), Style::default().fg(DIM)),
                            Span::styled("- ", Style::default().fg(diff_del).bold()),
                            Span::styled(truncated, Style::default().fg(diff_del)),
                        ]));
                        line_num += 1;
                    }
                    line_num = 1;
                    for line in new_str.lines() {
                        let truncated: String = line.chars().take(diff_content_w).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!("{:>4} ", line_num), Style::default().fg(DIM)),
                            Span::styled("+ ", Style::default().fg(diff_add).bold()),
                            Span::styled(truncated, Style::default().fg(diff_add)),
                        ]));
                        line_num += 1;
                    }
                }
            }
            ContentBlock::Text(text) => {
                let text = text.trim();
                if text.is_empty() { continue; }

                // Skip system noise
                if text.contains("<system-reminder>") || text.contains("<command-name>")
                    || text.starts_with("Plan mode is active")
                    || text.contains("<local-command")
                    || text.contains("Caveat: The messages below")
                {
                    continue;
                }

                let badge_spans: Vec<Span<'static>> = if m.role == "user" {
                    let c = Color::Rgb(60, 120, 190);
                    vec![
                        Span::raw("  "),
                        Span::styled("\u{E0B6}", Style::default().fg(c)),
                        Span::styled(" me ", Style::default().fg(Color::White).bg(c).bold()),
                        Span::styled("\u{E0B4}", Style::default().fg(c)),
                    ]
                } else {
                    let c = Color::Rgb(40, 140, 70);
                    vec![
                        Span::styled("\u{E0B6}", Style::default().fg(c)),
                        Span::styled("claude", Style::default().fg(Color::White).bg(c).bold()),
                        Span::styled("\u{E0B4}", Style::default().fg(c)),
                    ]
                };

                lines.extend(render_markdown_to_lines(text, text_w, badge_spans));
                lines.push(Line::from(""));
            }
        }
    }

    // ── Search highlighting ──
    if !app.chat_search_query.is_empty() {
        let current_line = app.chat_search_matches.get(app.chat_search_current).copied();
        let match_indices = highlight_search_matches(&mut lines, &app.chat_search_query, current_line);
        app.chat_search_matches = match_indices;
        // Re-clamp current if matches changed
        if !app.chat_search_matches.is_empty() && app.chat_search_current >= app.chat_search_matches.len() {
            app.chat_search_current = 0;
        }
    } else {
        app.chat_search_matches.clear();
    }

    let chat_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(" Chat ")
        .title_style(Style::default().fg(Color::White).bold());

    let _inner_w = chat_block.inner(chat_area).width as usize;
    let inner_h = chat_block.inner(chat_area).height as usize;

    // Store for scroll_to_search_match
    let total_lines = lines.len();
    app.chat_total_lines = total_lines;
    app.chat_inner_h = inner_h;

    let max_scroll = total_lines.saturating_sub(inner_h);
    app.chat_max_scroll = max_scroll;

    // detail_scroll: 0 = bottom (latest), higher = scrolled up
    let scroll_y = (max_scroll.saturating_sub(detail_scroll) as u16).min(total_lines as u16);
    app.chat_scroll_y.set(scroll_y);

    let scroll_label = if !app.chat_search_query.is_empty() {
        let total = app.chat_search_matches.len();
        if total > 0 {
            format!(" Chat [{}/{}] ", app.chat_search_current + 1, total)
        } else {
            " Chat [no matches] ".to_string()
        }
    } else if detail_scroll > 0 {
        format!(" Chat [{}↑] ", detail_scroll)
    } else {
        format!(" Chat [{} msgs] ", lines.iter().filter(|l| !l.spans.is_empty()).count())
    };

    let chat = Paragraph::new(lines)
        .scroll((scroll_y, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(scroll_label)
                .title_style(Style::default().fg(Color::White).bold()),
        );
    f.render_widget(chat, chat_area);

    // Scrollbar
    {
        let mut scrollbar_state = ratatui::widgets::ScrollbarState::new(max_scroll)
            .position(scroll_y as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(DIM))
            .thumb_style(Style::default().fg(LABEL));
        f.render_stateful_widget(scrollbar, chat_area, &mut scrollbar_state);
    }

    // Mascot — rendered by the state machine
    mascot.render(f, char_area);
}
