mod session;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    // Load sessions
    let store = SessionStore::load();
    let mut app = App::new(store);

    // Main loop — reload sessions every ~3 seconds (30 ticks at 100ms)
    'main: loop {
        if quit_signal.load(Ordering::Relaxed) {
            break;
        }
        app.tick += 1;
        app.mascot.update();

        if app.tick.is_multiple_of(30) {
            app.reload_sessions();
        }

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
                    match app.mode {
                        AppMode::List => match key.code {
                            KeyCode::Char('q') if app.search_query.is_empty() => break,
                            KeyCode::Esc => {
                                if !app.search_query.is_empty() {
                                    app.search_query.clear();
                                    app.update_filtered();
                                } else {
                                    break 'main;
                                }
                            }
                            KeyCode::Up => app.move_cursor(-1),
                            KeyCode::Down => app.move_cursor(1),
                            KeyCode::Left => {
                                app.list_info_tab = app.list_info_tab.saturating_sub(1);
                            }
                            KeyCode::Right => {
                                app.list_info_tab = (app.list_info_tab + 1).min(2);
                            }
                            KeyCode::Enter => {
                                if !app.filtered_indices.is_empty() {
                                    // Mark session as seen to dismiss waiting indicator
                                    if let Some(s) = app.selected_session() {
                                        app.seen_sessions.insert(s.id.clone());
                                    }
                                    app.mode = AppMode::Detail;
                                }
                            }
                            KeyCode::Char('X') => {
                                // Clear all waiting indicators
                                let ids: Vec<String> = app.filtered_indices.iter()
                                    .filter_map(|&idx| app.store.sessions.get(idx))
                                    .map(|s| s.id.clone())
                                    .collect();
                                for id in ids {
                                    app.seen_sessions.insert(id);
                                }
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
                                // Clean exit, then hand off terminal to claude --resume
                                if let Some(s) = app.selected_session() {
                                    let sid = s.id.clone();
                                    // Resolve cwd: expand ~ back to home dir
                                    let cwd = s.cwd.replace("~", &dirs::home_dir().unwrap_or_default().to_string_lossy());
                                    // Restore terminal
                                    disable_raw_mode()?;
                                    execute!(
                                        terminal.backend_mut(),
                                        crossterm::event::DisableMouseCapture,
                                        LeaveAlternateScreen
                                    )?;
                                    terminal.show_cursor()?;
                                    // Replace process with claude from the session's directory
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
                                app.move_cursor(-1);
                                app.detail_scroll = 0;
                                if let Some(s) = app.selected_session() {
                                    app.seen_sessions.insert(s.id.clone());
                                }
                            }
                            KeyCode::Right => {
                                app.move_cursor(1);
                                app.detail_scroll = 0;
                                if let Some(s) = app.selected_session() {
                                    app.seen_sessions.insert(s.id.clone());
                                }
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
