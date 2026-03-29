mod log;
mod session;
mod terminal;
mod ui;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind, MouseButton},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use session::SessionStore;
use ui::{App, AppMode};

/// Try to focus the terminal tab running a session, or open a new tab as fallback.
fn focus_or_open_session(app: &mut App) {
    let Some(s) = app.selected_session() else { return };
    let sid = s.id.clone();
    let title = s.title.clone();
    let turns = s.turns;
    let cwd = s.cwd.replace("~", &dirs::home_dir().unwrap_or_default().to_string_lossy());
    // Dismiss waiting indicator — user is engaging with this session
    app.seen_sessions.insert(sid.clone(), turns);

    cs_log!("focus_or_open: session={} title={} cwd={}", &sid[..sid.len().min(12)], title, cwd);

    if let Some(info) = app.process_map.get(&sid) {
        cs_log!("focus_or_open: found process pid={} tty={} confidence={:?} our_tty={:?}", info.pid, info.tty, info.confidence, app.our_tty);
        // Check if this session is running in our own terminal tab
        if let Some(ref our_tty) = app.our_tty {
            if info.tty == *our_tty {
                cs_log!("focus_or_open: same tty, skipping");
                app.status_message = Some(("Session is in this terminal".into(), std::time::Instant::now()));
                return;
            }
        }
        match crate::terminal::focus_tab_by_tty(&info.tty) {
            Ok(()) => {
                cs_log!("focus_or_open: focused tty={}", info.tty);
                app.status_message = Some(("Focused session tab".into(), std::time::Instant::now()));
            }
            Err(e) => {
                cs_log!("focus_or_open: focus failed ({}), opening new tab", e);
                match crate::terminal::open_in_new_tab(&sid, &cwd) {
                    Ok(()) => app.status_message = Some(("Opened in new tab".into(), std::time::Instant::now())),
                    Err(e) => app.status_message = Some((e, std::time::Instant::now())),
                }
            }
        }
    } else {
        cs_log!("focus_or_open: no process found, opening new tab. process_map has {} entries", app.process_map.len());
        match crate::terminal::open_in_new_tab(&sid, &cwd) {
            Ok(()) => app.status_message = Some(("Not running — opened new tab".into(), std::time::Instant::now())),
            Err(e) => app.status_message = Some((e, std::time::Instant::now())),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // CLI arg: --config-terminal
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--config-terminal") {
        terminal::run_config_terminal_flow();
        return Ok(());
    }

    // Setup terminal with mouse capture
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?; // clean slate — no artifacts from previous terminal

    // Handle SIGTERM (e.g. from `killall claude-stats`) gracefully so
    // LeaveAlternateScreen runs and terminal burn-in doesn't occur.
    let quit_signal = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&quit_signal))?;

    log::init();
    cs_log!("terminal size: {}x{}", crossterm::terminal::size().map(|s| s.0).unwrap_or(0), crossterm::terminal::size().map(|s| s.1).unwrap_or(0));

    // Load sessions
    let store = SessionStore::load();
    cs_log!("loaded {} sessions", store.sessions.len());
    let mut app = App::new(store);
    app.rebuild_display_rows();

    // Main loop — fast refresh every ~1s, full reload every ~5s
    'main: loop {
        if quit_signal.load(Ordering::Relaxed) {
            break;
        }
        app.tick += 1;
        app.mascot.update();

        if app.tick.is_multiple_of(50) {
            // Full reload every ~5s: re-scan files, pick up new/removed sessions
            app.reload_sessions();
            let session_quads: Vec<(String, String, i64, String)> = app.store.sessions.iter()
                .map(|s| (s.id.clone(), s.file_path.clone(), s.end_ts.map(|t| t.timestamp()).unwrap_or(0), s.title.clone()))
                .collect();
            app.process_map = terminal::scan_claude_processes(&session_quads);
        } else if app.tick.is_multiple_of(10) {
            // Fast refresh every ~1s: only read new bytes for waiting state
            app.fast_refresh();
        }

        ui::poll_mcp_result(&mut app);
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // chat_area_top is set inside draw_claude_animation() during draw

        // Drain ALL queued events — prevents scroll chunking
        while event::poll(std::time::Duration::from_millis(50))? {
            let evt = event::read()?;

            // Ctrl+C always quits
            if let Event::Key(key) = &evt {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break 'main;
                }
            }

            match &evt {
                Event::Key(key) => {
                    let mode_str = match app.mode { AppMode::List => "List", AppMode::Detail => "Detail" };
                    let mods = if key.modifiers.is_empty() { String::new() } else { format!("{:?}+", key.modifiers) };
                    cs_log!("key: {}{:?} mode={} tab={} archive={} sel={}", mods, key.code, mode_str, app.list_info_tab, app.viewing_archive, app.selected_ids.len());
                    match app.mode {
                        AppMode::List => match key.code {
                            KeyCode::Char('q') if app.search_query.is_empty() && !app.viewing_archive => break,
                            KeyCode::Char('q') if app.search_query.is_empty() && app.viewing_archive => {
                                app.viewing_archive = false;
                                app.update_filtered();
                            }
                            KeyCode::Esc => {
                                if !app.selected_ids.is_empty() {
                                    app.selected_ids.clear();
                                } else if !app.search_query.is_empty() {
                                    app.search_query.clear();
                                    app.update_filtered();
                                } else if app.viewing_archive {
                                    app.viewing_archive = false;
                                    app.update_filtered();
                                } else {
                                    break 'main;
                                }
                            }
                            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                                // Toggle selection on current row, then move up
                                if let Some(s) = app.selected_session() {
                                    let id = s.id.clone();
                                    if !app.selected_ids.remove(&id) {
                                        app.selected_ids.insert(id);
                                    }
                                }
                                app.move_cursor(-1);
                            }
                            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                                if let Some(s) = app.selected_session() {
                                    let id = s.id.clone();
                                    if !app.selected_ids.remove(&id) {
                                        app.selected_ids.insert(id);
                                    }
                                }
                                app.move_cursor(1);
                            }
                            KeyCode::Up => app.move_cursor(-1),
                            KeyCode::Down => app.move_cursor(1),
                            KeyCode::Left => {
                                app.list_info_tab = app.list_info_tab.saturating_sub(1);
                                if app.list_info_tab == 4 && app.mcp_statuses.is_empty() {
                                    ui::trigger_mcp_check(&mut app);
                                }
                            }
                            KeyCode::Right => {
                                app.list_info_tab = (app.list_info_tab + 1).min(4);
                                if app.list_info_tab == 4 && app.mcp_statuses.is_empty() {
                                    ui::trigger_mcp_check(&mut app);
                                }
                            }
                            KeyCode::Enter => {
                                match app.display_rows.get(app.cursor) {
                                    Some(ui::DisplayRow::AgentSummary { parent_id, .. }) => {
                                        let pid = parent_id.clone();
                                        if !app.expanded_parents.insert(pid.clone()) {
                                            app.expanded_parents.remove(&pid);
                                        }
                                        app.rebuild_display_rows();
                                    }
                                    Some(ui::DisplayRow::Session(_)) => {
                                        let title = app.selected_session().map(|s| s.title.clone()).unwrap_or_default();
                                        cs_log!("mode: List → Detail ({})", title);
                                        app.mode = AppMode::Detail;
                                    }
                                    None => {}
                                }
                            }
                            KeyCode::Char('X') => {
                                // Clear indicator on selected row
                                if let Some(s) = app.selected_session() {
                                    app.seen_sessions.insert(s.id.clone(), s.turns);
                                }
                            }
                            KeyCode::Char('C') => {
                                // Clear all indicators
                                let entries: Vec<(String, usize)> = app.filtered_indices.iter()
                                    .filter_map(|&idx| app.store.sessions.get(idx))
                                    .map(|s| (s.id.clone(), s.turns))
                                    .collect();
                                for (id, turns) in entries {
                                    app.seen_sessions.insert(id, turns);
                                }
                            }
                            KeyCode::Char('K') if app.search_query.is_empty() => {
                                focus_or_open_session(&mut app);
                            }
                            KeyCode::Char('R') if app.list_info_tab == 4 && app.search_query.is_empty() => {
                                ui::trigger_mcp_check(&mut app);
                            }
                            KeyCode::Char('A') if app.list_info_tab == 3 && !app.viewing_archive => {
                                let mut to_archive: Vec<String> = Vec::new();
                                if !app.selected_ids.is_empty() {
                                    to_archive.extend(app.selected_ids.drain());
                                } else if let Some(s) = app.selected_session() {
                                    to_archive.push(s.id.clone());
                                }
                                // Also archive child agents by scanning filesystem
                                let children: Vec<String> = to_archive.iter()
                                    .flat_map(|id| ui::find_child_agent_ids(id))
                                    .collect();
                                let count = to_archive.len();
                                for id in to_archive.into_iter().chain(children) {
                                    app.archived_ids.insert(id);
                                }
                                cs_log!("archive: added {} sessions (+children), total={}", count, app.archived_ids.len());
                                ui::save_archive(&app.archived_ids);
                                app.update_filtered();
                            }
                            KeyCode::Char('V') if app.list_info_tab == 3 && !app.viewing_archive => {
                                cs_log!("archive: entering view ({} archived)", app.archived_ids.len());
                                app.viewing_archive = true;
                                app.update_filtered();
                            }
                            KeyCode::Char('R') if app.viewing_archive => {
                                let mut to_unarchive: Vec<String> = Vec::new();
                                if !app.selected_ids.is_empty() {
                                    to_unarchive.extend(app.selected_ids.drain());
                                } else if let Some(s) = app.selected_session() {
                                    to_unarchive.push(s.id.clone());
                                }
                                // Also unarchive child agents by scanning filesystem
                                let children: Vec<String> = to_unarchive.iter()
                                    .flat_map(|id| ui::find_child_agent_ids(id))
                                    .collect();
                                let count = to_unarchive.len();
                                for id in to_unarchive.into_iter().chain(children) {
                                    app.archived_ids.remove(&id);
                                }
                                cs_log!("archive: removed {} sessions (+children), total={}", count, app.archived_ids.len());
                                ui::save_archive(&app.archived_ids);
                                app.update_filtered();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.update_filtered();
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.update_filtered();
                            }
                            _ => {}
                        },
                        AppMode::Detail if app.chat_search_active => match key.code {
                            KeyCode::Char(c) => {
                                app.chat_search_query.push(c);
                            }
                            KeyCode::Backspace => {
                                app.chat_search_query.pop();
                            }
                            KeyCode::Enter => {
                                app.chat_search_active = false;
                                // Focus first match (set by render)
                                if !app.chat_search_matches.is_empty() {
                                    app.chat_search_current = 0;
                                    app.scroll_to_search_match();
                                }
                            }
                            KeyCode::Esc => {
                                app.chat_search_active = false;
                                app.chat_search_query.clear();
                                app.chat_search_matches.clear();
                                app.chat_search_current = 0;
                            }
                            _ => {}
                        },
                        AppMode::Detail => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                                if !app.chat_search_query.is_empty() {
                                    // Clear search first
                                    app.chat_search_query.clear();
                                    app.chat_search_matches.clear();
                                    app.chat_search_current = 0;
                                } else if app.chat_fullscreen {
                                    app.chat_fullscreen = false;
                                } else {
                                    // Dismiss indicator on the session we're leaving
                                    if let Some(s) = app.selected_session() {
                                        app.seen_sessions.insert(s.id.clone(), s.turns);
                                    }
                                    cs_log!("mode: Detail → List");
                                    app.mode = AppMode::List;
                                    app.detail_scroll = 0;
                                }
                            }
                            KeyCode::Char('/') => {
                                app.chat_search_active = true;
                                app.chat_search_query.clear();
                                app.chat_search_matches.clear();
                                app.chat_search_current = 0;
                            }
                            KeyCode::Char('n') => {
                                if !app.chat_search_matches.is_empty() {
                                    app.chat_search_current = (app.chat_search_current + 1) % app.chat_search_matches.len();
                                    app.scroll_to_search_match();
                                }
                            }
                            KeyCode::Char('N') => {
                                if !app.chat_search_matches.is_empty() {
                                    if app.chat_search_current == 0 {
                                        app.chat_search_current = app.chat_search_matches.len() - 1;
                                    } else {
                                        app.chat_search_current -= 1;
                                    }
                                    app.scroll_to_search_match();
                                }
                            }
                            KeyCode::Char('f') => {
                                app.chat_fullscreen = !app.chat_fullscreen;
                            }
                            KeyCode::Char('c') => {
                                // Open session in a new terminal tab (claude-stats stays running)
                                if let Some(s) = app.selected_session() {
                                    let sid = s.id.clone();
                                    let turns = s.turns;
                                    let cwd = s.cwd.replace("~", &dirs::home_dir().unwrap_or_default().to_string_lossy());
                                    app.seen_sessions.insert(sid.clone(), turns);
                                    match crate::terminal::open_in_new_tab(&sid, &cwd) {
                                        Ok(()) => {
                                            app.status_message = Some(("Opened in new tab".into(), std::time::Instant::now()));
                                        }
                                        Err(e) => {
                                            app.status_message = Some((e, std::time::Instant::now()));
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('K') | KeyCode::Char('k') => {
                                focus_or_open_session(&mut app);
                            }
                            KeyCode::Char('C') => {
                                // Replace this process with claude --resume (legacy behavior)
                                if let Some(s) = app.selected_session() {
                                    let sid = s.id.clone();
                                    let turns = s.turns;
                                    let cwd = s.cwd.replace("~", &dirs::home_dir().unwrap_or_default().to_string_lossy());
                                    app.seen_sessions.insert(sid.clone(), turns);
                                    disable_raw_mode()?;
                                    execute!(
                                        terminal.backend_mut(),
                                        crossterm::event::DisableMouseCapture,
                                        LeaveAlternateScreen
                                    )?;
                                    terminal.show_cursor()?;
                                    // Unix exec: replaces this process entirely
                                    use std::os::unix::process::CommandExt;
                                    let e = std::process::Command::new("claude")
                                        .arg("--resume")
                                        .arg(sid)
                                        .current_dir(&cwd)
                                        .exec();
                                    eprintln!("Failed to launch claude: {}", e);
                                    std::process::exit(1);
                                }
                            }
                            KeyCode::Up => {
                                app.detail_scroll = (app.detail_scroll + 1).min(app.chat_max_scroll);
                                app.mascot.on_scroll();
                            }
                            KeyCode::Down => {
                                app.detail_scroll = app.detail_scroll.saturating_sub(1);
                                app.mascot.on_scroll();
                            }
                            KeyCode::PageUp => {
                                app.detail_scroll = (app.detail_scroll + 10).min(app.chat_max_scroll);
                                app.mascot.on_scroll();
                            }
                            KeyCode::PageDown => {
                                app.detail_scroll = app.detail_scroll.saturating_sub(10);
                                app.mascot.on_scroll();
                            }
                            KeyCode::Home => {
                                app.detail_scroll = app.chat_max_scroll;
                            }
                            KeyCode::End => {
                                app.detail_scroll = 0;
                            }
                            KeyCode::Enter => {
                                // Toggle all collapsed/expanded tool summaries
                                if app.expanded_msgs.is_empty() {
                                    // Expand all — collect indices first to avoid borrow conflict
                                    let indices: Vec<usize> = app.selected_session()
                                        .map(|s| s.messages.iter().enumerate()
                                            .filter(|(_, m)| matches!(m.block, session::ContentBlock::ToolUse { .. }))
                                            .map(|(i, _)| i)
                                            .collect())
                                        .unwrap_or_default();
                                    for i in indices {
                                        app.expanded_msgs.insert(i);
                                    }
                                } else {
                                    app.expanded_msgs.clear();
                                }
                            }
                            KeyCode::Left => {
                                // Dismiss indicator on session we're leaving
                                if let Some(s) = app.selected_session() {
                                    app.seen_sessions.insert(s.id.clone(), s.turns);
                                }
                                app.move_cursor_skip_agents(-1);
                                app.detail_scroll = 0;
                            }
                            KeyCode::Right => {
                                // Dismiss indicator on session we're leaving
                                if let Some(s) = app.selected_session() {
                                    app.seen_sessions.insert(s.id.clone(), s.turns);
                                }
                                app.move_cursor_skip_agents(1);
                                app.detail_scroll = 0;
                            }
                            KeyCode::Char('m') => {
                                // Toggle mouse capture for text selection
                                app.mouse_captured = !app.mouse_captured;
                                if app.mouse_captured {
                                    execute!(
                                        terminal.backend_mut(),
                                        crossterm::event::EnableMouseCapture
                                    )?;
                                } else {
                                    execute!(
                                        terminal.backend_mut(),
                                        crossterm::event::DisableMouseCapture
                                    )?;
                                }
                            }
                            _ => {}
                        },
                    }
                }
                Event::Mouse(mouse) => {
                    match app.mode {
                        AppMode::List => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp => app.move_cursor(-1),
                                MouseEventKind::ScrollDown => app.move_cursor(1),
                                MouseEventKind::Down(MouseButton::Left) => {
                                    // Table content starts 2 rows below table widget top (border + header)
                                    let table_content_top = app.list_table_top + 2;
                                    if mouse.row >= table_content_top {
                                        let clicked_row = app.list_offset + (mouse.row - table_content_top) as usize;
                                        if clicked_row < app.display_rows.len() {
                                            // Double-click detection: same row within 400ms
                                            let is_double = app.last_click
                                                .map(|(t, r, _)| r == mouse.row && t.elapsed().as_millis() < 400)
                                                .unwrap_or(false);
                                            app.last_click = Some((std::time::Instant::now(), mouse.row, mouse.column));
                                            let row_type = match app.display_rows.get(clicked_row) {
                                                Some(ui::DisplayRow::Session(idx)) => {
                                                    let has_agents = app.agent_counts.contains_key(&app.store.sessions[*idx].id);
                                                    if has_agents { "parent" } else { "session" }
                                                }
                                                Some(ui::DisplayRow::AgentSummary { .. }) => "summary",
                                                None => "none",
                                            };
                                            cs_log!("click: col={} row={} type={} dbl={}", mouse.column, clicked_row, row_type, is_double);

                                            app.cursor = clicked_row;

                                            // Left hitbox (cols 0-4, marker/arrow area): toggle agents
                                            if mouse.column <= 4 {
                                                match app.display_rows.get(clicked_row) {
                                                    Some(ui::DisplayRow::AgentSummary { parent_id, .. }) => {
                                                        let pid = parent_id.clone();
                                                        if !app.expanded_parents.insert(pid.clone()) {
                                                            app.expanded_parents.remove(&pid);
                                                        }
                                                        app.rebuild_display_rows();
                                                    }
                                                    Some(ui::DisplayRow::Session(idx)) => {
                                                        let sid = app.store.sessions[*idx].id.clone();
                                                        if app.agent_counts.contains_key(&sid) {
                                                            if !app.expanded_parents.insert(sid.clone()) {
                                                                app.expanded_parents.remove(&sid);
                                                            }
                                                            app.rebuild_display_rows();
                                                        }
                                                    }
                                                    None => {}
                                                }
                                            }
                                            // Right hitbox (cols 5+): double-click to inspect
                                            else if is_double {
                                                if let Some(ui::DisplayRow::Session(_)) = app.display_rows.get(clicked_row) {
                                                    let title = app.selected_session().map(|s| s.title.clone()).unwrap_or_default();
                                                    cs_log!("mode: List → Detail (double-click: {})", title);
                                                    app.mode = AppMode::Detail;
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        AppMode::Detail => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    app.detail_scroll = (app.detail_scroll + 4).min(app.chat_max_scroll);
                                    app.mascot.on_scroll();
                                }
                                MouseEventKind::ScrollDown => {
                                    app.detail_scroll = app.detail_scroll.saturating_sub(4);
                                    app.mascot.on_scroll();
                                }
                                MouseEventKind::Down(MouseButton::Left) => {
                                    // Use pre-recorded clickable line positions
                                    if mouse.row > app.chat_area_top {
                                        let scroll_y = app.chat_scroll_y.get() as usize;
                                        let click_line = scroll_y + (mouse.row - app.chat_area_top - 1) as usize;

                                        // Find closest clickable line within 2 rows
                                        let clickables = app.clickable_lines.borrow();
                                        let mut best: Option<usize> = None;
                                        let mut best_dist = 3usize;
                                        for &(line_idx, msg_idx) in clickables.iter() {
                                            let dist = click_line.abs_diff(line_idx);
                                            if dist < best_dist {
                                                best_dist = dist;
                                                best = Some(msg_idx);
                                            }
                                        }
                                        drop(clickables);

                                        if let Some(msg_idx) = best {
                                            if app.expanded_msgs.contains(&msg_idx) {
                                                // Collapsing — reduce scroll by the diff lines
                                                if let Some(s) = app.selected_session() {
                                                    if let session::ContentBlock::ToolUse { old_str, new_str, .. } = &s.messages[msg_idx].block {
                                                        let diff_lines = old_str.lines().count() + new_str.lines().count() + 1;
                                                        app.detail_scroll = app.detail_scroll.saturating_sub(diff_lines);
                                                    }
                                                }
                                                app.expanded_msgs.remove(&msg_idx);
                                            } else {
                                                // Expanding — increase scroll so view stays anchored
                                                if let Some(s) = app.selected_session() {
                                                    if let session::ContentBlock::ToolUse { old_str, new_str, .. } = &s.messages[msg_idx].block {
                                                        let diff_lines = old_str.lines().count() + new_str.lines().count() + 1;
                                                        app.detail_scroll = (app.detail_scroll + diff_lines).min(app.chat_max_scroll);
                                                    }
                                                }
                                                app.expanded_msgs.insert(msg_idx);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Restore terminal — disable mouse before leaving
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(())
}
