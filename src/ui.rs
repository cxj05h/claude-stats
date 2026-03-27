use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{prelude::*, widgets::*};
use std::time::{Duration, Instant};

use crate::session::{fmt_ago, fmt_duration, fmt_tokens, short_model, Session, SessionStore};

use crate::session::context_window_for_model;

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

pub struct App {
    pub store: SessionStore,
    pub mode: AppMode,
    pub cursor: usize,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub list_offset: usize,
    pub tick: u64,
    pub detail_scroll: usize,
    pub chat_fullscreen: bool,
    pub list_info_tab: usize,
    pub mascot: Mascot,
    pub expanded_msgs: std::collections::HashSet<usize>,
    pub tool_summary_indices: Vec<usize>,  // msg indices that have tool summaries
    pub chat_area_top: u16,
    pub chat_scroll_y: std::cell::Cell<u16>,
    pub clickable_lines: std::cell::RefCell<Vec<(usize, usize)>>, // (line_index, msg_index) for ToolUse
    matcher: SkimMatcherV2,
}

impl App {
    pub fn new(store: SessionStore) -> Self {
        let count = store.sessions.len();
        let filtered_indices: Vec<usize> = (0..count).collect();
        App {
            store,
            mode: AppMode::List,
            cursor: 0,
            search_query: String::new(),
            filtered_indices,
            list_offset: 0,
            tick: 0,
            detail_scroll: 0,
            chat_fullscreen: false,
            list_info_tab: 0,
            mascot: Mascot::new(),
            expanded_msgs: std::collections::HashSet::new(),
            tool_summary_indices: Vec::new(),
            chat_area_top: 0,
            chat_scroll_y: std::cell::Cell::new(0),
            clickable_lines: std::cell::RefCell::new(Vec::new()),
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let len = self.filtered_indices.len();
        if delta < 0 {
            self.cursor = self.cursor.saturating_sub((-delta) as usize);
        } else {
            self.cursor = (self.cursor + delta as usize).min(len - 1);
        }
    }

    pub fn update_filtered(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices = (0..self.store.sessions.len().min(30)).collect();
        } else {
            let query = &self.search_query;
            let mut scored: Vec<(i64, usize)> = self
                .store
                .sessions
                .iter()
                .enumerate()
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
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.filtered_indices
            .get(self.cursor)
            .and_then(|&idx| self.store.sessions.get(idx))
    }

    pub fn reload_sessions(&mut self) {
        // Remember currently selected session ID to restore position
        let selected_id = self.selected_session().map(|s| s.id.clone());

        self.store = SessionStore::load();
        self.update_filtered();

        // Try to restore cursor to same session
        if let Some(id) = selected_id {
            for (i, &idx) in self.filtered_indices.iter().enumerate() {
                if self.store.sessions.get(idx).map(|s| &s.id) == Some(&id) {
                    self.cursor = i;
                    break;
                }
            }
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    match app.mode {
        AppMode::List => draw_list(f, app),
        AppMode::Detail => draw_detail(f, app),
    }
}

fn draw_list(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(2), // search
            Constraint::Min(10),  // table
            Constraint::Length(2), // info bar
            Constraint::Length(1), // footer
        ])
        .split(area);

    // Header
    let header = Paragraph::new(Line::from(vec![
        Span::styled("  Claude Stats ", Style::default().bold().fg(Color::White)),
        Span::styled(
            format!("  {} sessions", app.filtered_indices.len()),
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
    let visible_height = chunks[2].height.saturating_sub(3) as usize; // header + borders
    if app.cursor >= app.list_offset + visible_height {
        app.list_offset = app.cursor + 1 - visible_height;
    }
    if app.cursor < app.list_offset {
        app.list_offset = app.cursor;
    }

    let current_sid = app.store.current_session_id.as_deref();

    let header_cells = [
        "", "Title", "Model", "Effort", "Turns", "Tools", "MCPs",
        "Out Tkns", "Context", "Duration", "When",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(LABEL).bold()))
    .collect::<Vec<_>>();

    let header_row = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .filtered_indices
        .iter()
        .enumerate()
        .skip(app.list_offset)
        .take(visible_height)
        .map(|(i, &idx)| {
            let s = &app.store.sessions[idx];
            let is_selected = i == app.cursor;
            let is_live = current_sid.map(|c| c == s.id).unwrap_or(false);

            let marker = if is_selected {
                "▶"
            } else if is_live {
                "●"
            } else {
                " "
            };
            let marker_style = if is_selected {
                Style::default().fg(FOOTER_KEY).bold()
            } else if is_live {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            let is_agent = s.parent_session_id.is_some();
            let raw_title = if is_agent {
                format!("⤷ {}", s.title)
            } else {
                s.title.clone()
            };
            let title = if raw_title.len() > 28 {
                format!("{}…", &raw_title[..27])
            } else {
                raw_title
            };
            let title_style = if is_selected {
                Style::default().fg(Color::White).bold()
            } else if is_live {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            let model = short_model(&s.model);

            let effort = if !s.effort_changes.is_empty() {
                let e = &s.effort_changes.last().unwrap().1;
                e[..3].to_uppercase()
            } else {
                app.store.current_effort[..3].to_uppercase()
            };

            let dur = match (s.start_ts, s.end_ts) {
                (Some(start), Some(end)) => {
                    fmt_duration((end - start).num_seconds() as f64)
                }
                _ => String::new(),
            };

            let when = s.end_ts.as_ref().map(fmt_ago).unwrap_or_default();

            let ctx = if s.last_context_read > 0 {
                let cw = context_window_for_model(&s.model);
                let pct = (s.last_context_read as f64 / cw as f64 * 100.0).min(100.0);
                format!("{:.0}%", pct)
            } else {
                String::new()
            };

            let mcp_str: String = s
                .mcp_tools
                .keys()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");

            let row_style = if is_selected {
                Style::default().bg(SEL_BG)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(marker).style(marker_style),
                Cell::from(title).style(title_style),
                Cell::from(model).style(Style::default().fg(
                    if s.model.contains("opus") { Color::Magenta } else { Color::Cyan }
                )),
                Cell::from(effort).style(Style::default().fg(Color::Yellow)),
                Cell::from(s.turns.to_string()),
                Cell::from(s.tool_calls.to_string()),
                Cell::from(mcp_str).style(Style::default().fg(Color::Blue)),
                Cell::from(fmt_tokens(s.total_output)).style(Style::default().fg(Color::Green)),
                Cell::from(ctx),
                Cell::from(dur).style(Style::default().fg(LABEL)),
                Cell::from(when).style(Style::default().fg(LABEL)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Min(20),
        Constraint::Length(12),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header_row)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(" Sessions ")
                .title_style(Style::default().fg(Color::White).bold()),
        );
    f.render_widget(table, chunks[2]);

    // Info bar — shows details for selected session based on tab
    if let Some(&idx) = app.filtered_indices.get(app.cursor) {
        if let Some(s) = app.store.sessions.get(idx) {
            let tab_labels = ["MCPs", "Path", "Models"];
            let mut tab_spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
            for (i, label) in tab_labels.iter().enumerate() {
                if i == app.list_info_tab {
                    tab_spans.push(Span::styled(format!("[{}]", label), Style::default().fg(Color::White).bold()));
                } else {
                    tab_spans.push(Span::styled(format!(" {} ", label), Style::default().fg(DIM)));
                }
            }
            tab_spans.push(Span::styled("  ", Style::default()));

            let detail_text = match app.list_info_tab {
                0 => {
                    if s.mcp_tools.is_empty() {
                        "No MCPs used".to_string()
                    } else {
                        s.mcp_tools.iter()
                            .map(|(name, count)| format!("{} ×{}", name, count))
                            .collect::<Vec<_>>()
                            .join("  ")
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

            tab_spans.push(Span::styled(detail_text, Style::default().fg(PREVIEW)));
            if !parent_info.is_empty() {
                tab_spans.push(Span::styled(parent_info, Style::default().fg(Color::Rgb(140, 120, 180))));
            }

            f.render_widget(Paragraph::new(vec![
                Line::from(tab_spans),
            ]), chunks[3]);
        }
    }

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("navigate  ", Style::default().fg(LABEL)),
        Span::styled("←→ ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("info tab  ", Style::default().fg(LABEL)),
        Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("inspect  ", Style::default().fg(LABEL)),
        Span::styled("type ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("search  ", Style::default().fg(LABEL)),
        Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("quit  ", Style::default().fg(LABEL)),
        Span::styled("● ", Style::default().fg(Color::Green)),
        Span::styled("active", Style::default().fg(LABEL)),
    ]));
    f.render_widget(footer, chunks[4]);
}

fn draw_detail(f: &mut Frame, app: &mut App) {
    let session = match app.selected_session() {
        Some(s) => s.clone(),
        None => return,
    };

    let area = f.area();
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
        let title_text = if is_live {
            format!(" ● {} (live)", session.title)
        } else {
            format!(" {}", session.title)
        };
        f.render_widget(Paragraph::new(Line::from(
            Span::styled(title_text, Style::default().bold().fg(if is_live { Color::Green } else { Color::White }))
        )), chunks[0]);

        // Chat + mascot
        draw_claude_animation(f, chunks[1], &session, app);

        // Footer
        f.render_widget(Paragraph::new(Line::from(vec![
            Span::styled(" f ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("exit fullscreen  ", Style::default().fg(LABEL)),
            Span::styled("↑↓ ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("scroll  ", Style::default().fg(LABEL)),
            Span::styled("Enter ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("expand/collapse  ", Style::default().fg(LABEL)),
            Span::styled("Esc ", Style::default().fg(FOOTER_KEY).bold()),
            Span::styled("back", Style::default().fg(LABEL)),
        ])), chunks[2]);
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
    let title_text = if is_live {
        format!("  ● {} (live)", session.title)
    } else {
        format!("  {}", session.title)
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(title_text, Style::default().bold().fg(if is_live { Color::Green } else { Color::White })),
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

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("scroll ", Style::default().fg(LABEL)),
        Span::styled("f", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("ullscreen ", Style::default().fg(LABEL)),
        Span::styled("c", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("laude ", Style::default().fg(LABEL)),
        Span::styled("o", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("pen ", Style::default().fg(LABEL)),
        Span::styled("←→", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("nav ", Style::default().fg(LABEL)),
        Span::styled("Esc", Style::default().fg(FOOTER_KEY).bold()),
        Span::styled("back", Style::default().fg(LABEL)),
    ]));
    f.render_widget(footer, chunks[4]);
}


/// Parse inline markdown: **bold** and `code` into styled spans
fn parse_inline_md<'a>(text: &'a str, base_style: Style) -> Vec<Span<'a>> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Look for **bold** or `code`
        if let Some(pos) = remaining.find("**") {
            if pos > 0 {
                spans.push(Span::styled(&remaining[..pos], base_style));
            }
            let after = &remaining[pos + 2..];
            if let Some(end) = after.find("**") {
                spans.push(Span::styled(&after[..end], base_style.bold()));
                remaining = &after[end + 2..];
            } else {
                spans.push(Span::styled(&remaining[pos..], base_style));
                break;
            }
        } else if let Some(pos) = remaining.find('`') {
            if pos > 0 {
                spans.push(Span::styled(&remaining[..pos], base_style));
            }
            let after = &remaining[pos + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(
                    &after[..end],
                    Style::default().fg(Color::Rgb(180, 140, 200)),
                ));
                remaining = &after[end + 1..];
            } else {
                spans.push(Span::styled(&remaining[pos..], base_style));
                break;
            }
        } else {
            spans.push(Span::styled(remaining, base_style));
            break;
        }
    }
    spans
}

fn draw_claude_animation(f: &mut Frame, area: Rect, session: &Session, app: &App) {
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

    // ── Chat window: preserve formatting, render markdown-like content ──
    app.clickable_lines.borrow_mut().clear();
    let chat_w = chat_area.width.saturating_sub(2) as usize; // inner width minus borders
    let indent_w = 10usize; // "     me ▸ " or " claude ▸ " prefix width
    let text_w = chat_w.saturating_sub(indent_w); // available width for text content
    let mut lines: Vec<Line> = Vec::new();
    let code_color = Color::Rgb(180, 140, 200);  // purple for code
    let diff_add = Color::Rgb(80, 200, 80);       // green for +
    let diff_del = Color::Rgb(200, 80, 80);       // red for -
    let heading_color = Color::Rgb(200, 200, 220); // bright for headers
    let bullet_color = Color::Rgb(100, 180, 220);  // cyan for bullets

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
                for line in result.lines().take(5) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled("            └ ", Style::default().fg(tool_dim)),
                            Span::styled(trimmed.to_string(), Style::default().fg(tool_dim)),
                        ]));
                    }
                }
            }
            ContentBlock::ToolUse { name, summary, old_str, new_str } => {
                // Green dot + tool name like Claude Code
                let display_name = if name == "Edit" || name == "Write" {
                    format!("Update({})", summary)
                } else {
                    format!("{}({})", name, summary)
                };

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
                    let line_num_w = 4; // width for line numbers
                    lines.push(Line::from(vec![
                        Span::styled("            └ ", Style::default().fg(tool_dim)),
                        Span::styled(format!("Added {} lines, removed {} lines", add_count, del_count), Style::default().fg(tool_dim)),
                    ]));
                    let mut line_num = 1usize;
                    for line in old_str.lines() {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{:>4} ", line_num), Style::default().fg(DIM)),
                            Span::styled("- ", Style::default().fg(diff_del).bold()),
                            Span::styled(line.to_string(), Style::default().fg(diff_del)),
                        ]));
                        line_num += 1;
                    }
                    line_num = 1;
                    for line in new_str.lines() {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{:>4} ", line_num), Style::default().fg(DIM)),
                            Span::styled("+ ", Style::default().fg(diff_add).bold()),
                            Span::styled(line.to_string(), Style::default().fg(diff_add)),
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

                let (prefix, prefix_color) = if m.role == "user" {
                    ("     me", Color::Rgb(100, 180, 220))
                } else {
                    (" claude", Color::Green)
                };

                let mut first_output_line = true;
                let mut in_code_block = false;

                for (li, text_line) in text.split('\n').enumerate() {
                    let trimmed = text_line.trim();

                    if trimmed.starts_with("```") {
                        in_code_block = !in_code_block;
                        let label = if in_code_block {
                            trimmed.strip_prefix("```").unwrap_or("──────")
                        } else { "──────" };
                        let indent = if first_output_line {
                            first_output_line = false;
                            vec![Span::styled(prefix, Style::default().fg(prefix_color).bold()), Span::styled(" ▸ ", Style::default().fg(DIM))]
                        } else {
                            vec![Span::styled("          ", Style::default().fg(DIM))]
                        };
                        let mut spans = indent;
                        spans.push(Span::styled(label.to_string(), Style::default().fg(code_color).italic()));
                        lines.push(Line::from(spans));
                        continue;
                    }

                    let pfx = if first_output_line {
                        first_output_line = false;
                        vec![Span::styled(prefix, Style::default().fg(prefix_color).bold()), Span::styled(" ▸ ", Style::default().fg(DIM))]
                    } else {
                        vec![Span::styled("          ", Style::default().fg(DIM))]
                    };

                    if in_code_block {
                        let mut s = pfx;
                        s.push(Span::styled(format!("  {}", text_line), Style::default().fg(code_color)));
                        lines.push(Line::from(s));
                    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                        let mut s = pfx;
                        s.push(Span::styled("• ", Style::default().fg(bullet_color)));
                        s.extend(parse_inline_md(&trimmed[2..], Style::default().fg(PREVIEW)));
                        lines.push(Line::from(s));
                    } else if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
                        let mut s = pfx;
                        s.extend(parse_inline_md(trimmed, Style::default().fg(heading_color).bold()));
                        lines.push(Line::from(s));
                    } else if trimmed.len() > 2 && trimmed.as_bytes()[0].is_ascii_digit() && trimmed.contains(". ") {
                        if let Some(dot_pos) = trimmed.find(". ") {
                            let mut s = pfx;
                            s.push(Span::styled(trimmed[..dot_pos + 2].to_string(), Style::default().fg(bullet_color)));
                            s.extend(parse_inline_md(&trimmed[dot_pos + 2..], Style::default().fg(PREVIEW)));
                            lines.push(Line::from(s));
                        }
                    } else if text_line.len() > text_w && text_w > 0 {
                        // Long line — manual wrap with hanging indent
                        let chars: Vec<char> = text_line.chars().collect();
                        let mut pos = 0;
                        let mut is_first_chunk = true;
                        while pos < chars.len() {
                            let end = (pos + text_w).min(chars.len());
                            let chunk: String = chars[pos..end].iter().collect();
                            let indent = if is_first_chunk {
                                is_first_chunk = false;
                                pfx.clone()
                            } else {
                                vec![Span::styled("          ", Style::default().fg(DIM))]
                            };
                            let mut s = indent;
                            s.push(Span::styled(chunk, Style::default().fg(PREVIEW)));
                            lines.push(Line::from(s));
                            pos = end;
                        }
                    } else {
                        let mut s = pfx;
                        s.extend(parse_inline_md(text_line, Style::default().fg(PREVIEW)));
                        lines.push(Line::from(s));
                    }
                }
                lines.push(Line::from(""));
            }
        }
    }

    let chat_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(" Chat ")
        .title_style(Style::default().fg(Color::White).bold());

    let inner_w = chat_block.inner(chat_area).width as usize;
    let inner_h = chat_block.inner(chat_area).height as usize;

    // Lines are pre-wrapped, so total = lines.len()
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(inner_h);

    // detail_scroll: 0 = bottom (latest), higher = scrolled up
    let scroll_y = (max_scroll.saturating_sub(detail_scroll) as u16).min(total_lines as u16);
    app.chat_scroll_y.set(scroll_y);

    let scroll_label = if detail_scroll > 0 {
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
