mod session;
mod ui;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind, MouseButton},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;

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

    // Load sessions
    let store = SessionStore::load();
    let mut app = App::new(store);

    // Main loop — reload sessions every ~3 seconds (30 ticks at 100ms)
    'main: loop {
        app.tick += 1;
        app.mascot.update();

        if app.tick % 30 == 0 {
            app.reload_sessions();
        }

        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Store chat area position for click detection
        if app.mode == AppMode::Detail {
            let h = terminal.size()?.height;
            let is_short = h < 40;
            let info_h: u16 = if is_short { 10 } else { 12 };
            let ctx_h: u16 = if is_short { 8 } else { 14 };
            app.chat_area_top = 2 + info_h + ctx_h; // header + info + context
        }

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
                                    app.mode = AppMode::Detail;
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
                        AppMode::Detail => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                                if app.chat_fullscreen {
                                    app.chat_fullscreen = false;
                                } else {
                                    app.mode = AppMode::List;
                                    app.detail_scroll = 0;
                                }
                            }
                            KeyCode::Char('f') => {
                                app.chat_fullscreen = !app.chat_fullscreen;
                            }
                            KeyCode::Char('o') => {
                                // Open JSONL file in a new window
                                if let Some(s) = app.selected_session() {
                                    let p = s.file_path.clone();
                                    let _ = std::process::Command::new("open")
                                        .arg(p)
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .spawn();
                                }
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
                                app.detail_scroll += 1;
                                app.mascot.on_scroll();
                            }
                            KeyCode::Down => {
                                app.detail_scroll = app.detail_scroll.saturating_sub(1);
                                app.mascot.on_scroll();
                            }
                            KeyCode::PageUp => {
                                app.detail_scroll += 10;
                                app.mascot.on_scroll();
                            }
                            KeyCode::PageDown => {
                                app.detail_scroll = app.detail_scroll.saturating_sub(10);
                                app.mascot.on_scroll();
                            }
                            KeyCode::Home => {
                                app.detail_scroll = usize::MAX;
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
                            }
                            KeyCode::Right => {
                                app.move_cursor(1);
                                app.detail_scroll = 0;
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
                                    app.detail_scroll += 4; // fast scroll
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
                                            let dist = if click_line >= line_idx {
                                                click_line - line_idx
                                            } else {
                                                line_idx - click_line
                                            };
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
                                                        app.detail_scroll += diff_lines;
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
