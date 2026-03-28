# claude-stats

A lightweight session tracking manager for Claude Code. Browse, search, and monitor every conversation from a standalone terminal dashboard.

![Rust](https://img.shields.io/badge/Rust-ratatui-orange)

![sessions](assets/sessions.png)
![stats-dashboard](assets/stats-dashboard.png)
![indicators](assets/indicators.png)
![search](assets/search.png)
![search-next](assets/search-next.png)
![fullscreen](assets/fullscreen.png)
![expand-code-changes](assets/expand-code-changes.png)

## Why claude-stats

Claude Code sessions pile up fast. You run dozens of conversations across projects, switch models mid-session, spawn agents, burn through context windows -- and there's no single place to see what happened, how much you used, or where you left off.

claude-stats gives you that visibility. It reads Claude Code's local session files and renders them in a fast, keyboard-driven TUI. No API calls, no setup, no config. Just run `claude-stats` and you're looking at your 40 most recent sessions with full token breakdowns, context usage, model history, and searchable chat logs.

Think of it as the missing management layer: a way to stay on top of your Claude Code usage without leaving the terminal.

## What You Can Do

### See which sessions need attention

Two indicators appear next to session titles when action is needed:

- **👋** -- Claude responded and is waiting for your input. Auto-clears after 1 hour or when you view the session details.
- **⏳** -- A permission prompt, question, or multiselect is waiting for your approval. The session title turns red. Clears automatically when you respond in Claude Code, or when you view the session details.

Both indicators appear on the sessions screen and the session details page. Press `X` to clear the indicator on the selected row, or `C` to clear all indicators at once. Agent sessions never show indicators.

### Find any session instantly

Start typing and claude-stats fuzzy-searches across session titles, project paths, and model names. Results filter in real time. No scrolling through history, no grepping JSONL files -- just type a few characters and the session you want floats to the top.

### Monitor context usage per session

The detail view shows exactly where your context window stands: a color-coded gradient bar (blue through green to red at 80%+), the raw token count and percentage, and a category breakdown showing how much context is consumed by system prompts, your messages, tool results, assistant output, and images. You can see at a glance whether a session has room to breathe or is about to hit the wall.

### Track token consumption

Every session shows input, output, cache read, and cache write tokens. The detail view breaks these out individually with a bold total. The Usage tab (cycle with `Left`/`Right` on the session list) shows weekly rollups per model -- how many Opus and Sonnet tokens you've burned through, total sessions and messages. Useful for staying aware of your consumption patterns.

### Jump into any session with one key

Press `K` on either the session list or detail view and claude-stats will **focus the terminal tab** where that session is already running. It scans running `claude` processes in the background and maps them to sessions via PID/TTY matching -- so when you press the key, the switch is instant. Sessions with a running process show a blue `◆` indicator in the list. If the session isn't running anywhere, `K` falls back to opening it in a new tab.

Press `c` in the detail view to always open a **new terminal tab** via `claude --resume`. Press `C` (shift) for the legacy behavior that replaces the current process. Auto-detects your terminal (iTerm2, Terminal.app, Warp, tmux, zellij). Run `claude-stats --config-terminal` to configure manually.

### Track agent sessions

Subagent sessions are automatically linked to their parent conversation. They appear in the list with a `⤷` prefix and show the parent session name in the info bar. You can inspect agent sessions the same way as any other -- full token usage, chat history, context breakdown. Useful for understanding what your spawned agents actually did.

### See git branch per session

The detail view shows which git branch (or worktree) each session was running on. Handy when you're working across multiple feature branches and need to remember which conversation was driving which branch.

### Follow model and effort changes

Sessions that switched models show the full timeline (e.g., Opus 4.5 -> Sonnet 4.5) in both the Models info tab and the detail view. Effort level (low/med/high/max) is tracked per session. Context window size adjusts automatically -- 1M for Opus, 200K for Sonnet/Haiku.

### Search within chat messages

Press `/` in the detail view to search inside the conversation. Matches are highlighted in yellow, the current match in orange. Use `n`/`N` to jump between results. The viewport auto-scrolls to center each match.

### Spot live sessions

The currently active session gets a green `●` indicator and highlighted row in the list. Waiting state updates refresh every ~1 second (incremental file reads), with a full session reload every ~5 seconds.

## How to Use

### Session list

Launch `claude-stats` and you'll see your 40 most recent sessions in a table:

| Column | Shows |
|--------|-------|
| Title | Session name (agents prefixed with `⤷`) |
| Model | Active model (Opus, Sonnet, Haiku) |
| Effort | Effort level (LOW/MED/HIGH/MAX) |
| Tokens | Total input + output tokens |
| Turns | Number of conversation turns |
| MCPs | Top 2 MCP servers used (GitHub, Notion, etc.) |
| When | Time since last activity (5m ago, 2h ago) |
| Duration | Total session time |

Navigate with `Up`/`Down`. The info bar below the table has three tabs you can cycle with `Left`/`Right`:

- **Branch** -- git branch or worktree for the session
- **Path** -- working directory where the session ran
- **Models** -- model transition timeline

### Detail view

Press `Enter` on any session to inspect it. The detail view has four panels:

- **Session info** -- model, effort, start time, duration, turn count, tool calls, MCP usage, git branch
- **Token usage** -- output, input, cache read, cache write, total, and last-turn breakdown
- **Context window** -- visual usage bar with color gradient, category breakdown with token estimates, remaining capacity
- **Chat history** -- full scrollable conversation with markdown rendering, code blocks, diff highlighting, and expandable tool calls

Press `f` to go fullscreen on the chat. Press `Enter` to expand/collapse all tool call diffs at once, or click individual ones. Use `Left`/`Right` to step through sessions without going back to the list.

### Quick resume

From the detail view, press `c` to open that session in a new terminal tab. claude-stats stays running. Press `C` (shift) to replace claude-stats with the session (original behavior). Both launch from the session's original working directory.

## Keybindings

### Session list

| Key | Action |
|-----|--------|
| `Up` / `Down` | Navigate sessions |
| `Left` / `Right` | Cycle info tabs (Branch / Path / Models) |
| `K` | Focus running session tab (or open new tab) |
| `X` | Clear indicator on selected row |
| `C` | Clear all waiting indicators |
| `Enter` | Open session detail |
| Type anything | Fuzzy search sessions |
| `Backspace` | Delete search character |
| `Esc` | Clear search / quit |
| `q` | Quit |

### Detail view

| Key | Action |
|-----|--------|
| `Up` / `Down` | Scroll chat |
| `PgUp` / `PgDn` | Scroll chat by 10 lines |
| `Home` / `End` | Jump to top / bottom |
| `Left` / `Right` | Previous / next session |
| `Enter` | Expand/collapse all tool diffs |
| `f` | Toggle fullscreen chat |
| `K` | Focus running session tab (or open new tab) |
| `c` | Open session in new terminal tab |
| `C` | Resume session here (replaces claude-stats) |
| `/` | Search within chat |
| `n` / `N` | Next / previous search match |
| `m` | Toggle mouse capture (for text selection) |
| `Esc` / `q` | Back to list |

## Install

### Homebrew (macOS / Linux)

```bash
brew install cxj05h/tap/claude-stats
```

### One-liner (macOS / Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/cxj05h/claude-stats/main/install.sh | bash
```

Installs to `~/.local/bin/claude-stats`. Make sure that directory is in your `PATH`.

Optionally add a short alias:

```bash
alias cs='claude-stats'
```

### Build from source

```bash
git clone https://github.com/cxj05h/claude-stats
cd claude-stats
cargo build --release
cp target/release/claude-stats ~/.local/bin/
```

Requires Rust 1.70+.

## Data Sources

claude-stats reads directly from Claude Code's local files -- no API calls needed:

- `~/.claude/projects/` -- session JSONL files
- `~/.claude/stats-cache.json` -- weekly token statistics
- `~/.claude/stats-config.json` -- plan type and terminal configuration
- `~/.claude/settings.json` -- effort level

Works with both subscription (Pro/Max) and API key authentication -- any login method that produces local session files.

## Dependencies

- [ratatui](https://github.com/ratatui/ratatui) -- terminal UI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) -- terminal manipulation
- [serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json) -- JSONL parsing
- [chrono](https://github.com/chronotope/chrono) -- timestamp handling
- [dirs](https://github.com/dirs-dev/dirs-rs) -- home directory resolution
- [fuzzy-matcher](https://github.com/lotabout/fuzzy-matcher) -- session search
