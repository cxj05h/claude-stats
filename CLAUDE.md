# claude-stats

## Project Goal

Build a native terminal dashboard (`claude-stats`) that gives full visibility into Claude Code usage — the same data you'd get from `/usage`, `/context`, and session history, but in a standalone TUI you can run from any directory.

## Current State

The Rust TUI is functional with session browsing, token usage, context breakdown, model/effort tracking, animated Claude mascot, scrollable chat history, and usage stats.

### Subscription Usage (API dead)

The OAuth usage endpoint (`api.anthropic.com/api/oauth/usage`) was disabled by Anthropic ("OAuth authentication is currently not supported"). The usage-cache.sh hook no longer works. An Ink (React for CLI) rewrite was attempted and abandoned — Ink can't do fixed-height scrollable regions or flicker-free full-screen rendering.

**Current approach**: Weekly token stats are computed locally from `~/.claude/stats-cache.json` (written by Claude Code). Accessible via the Usage tab (←→) in the session list. Shows plan type, weekly Opus/Sonnet tokens, total sessions/messages.

### Live Detection

"Live" session is detected by finding the session with the most recent `end_ts` within the last 10 minutes. No longer reads from `~/.claude/sessions/`.

### Agent Linking

Agent sessions (spawned subagents) are linked to their parent session via the file path structure: `projects/<project>/<parent-id>/subagents/agent-*.jsonl`. Agents are marked with `⤷` in the list and show their parent session name in the info bar.

## Key Files

- `Cargo.toml` — dependencies and project config
- `src/main.rs` — entry point, terminal setup, event loop, keybindings
- `src/session.rs` — JSONL parsing, Session/SessionStore structs, stats cache loading
- `src/ui.rs` — TUI rendering (App, Mascot, draw functions, layout, chat view)
- `.claude/skills/ready-ship/` — commit, build, and push workflow
- `.claude/skills/cs-feature/` — feature branch lifecycle management
- `~/.local/bin/claude-stats` — installed binary
- `~/.claude/stats-cache.json` — local usage statistics (written by Claude Code)
- `~/.claude/stats-config.json` — plan type (`{"plan": "max_20x"}`)

## Development

```bash
source ~/.cargo/env
cargo build --release
cp target/release/claude-stats ~/.local/bin/claude-stats
```

## Design Decisions

- **Rust + ratatui** over Ink/Python — smooth terminal rendering, no flicker, proper double-buffered output. Ink was tried and abandoned due to fundamental rendering limitations.
- **JSONL parsing** — reads Claude Code's session files directly, no API calls needed for session data
- **Local stats cache** — reads `~/.claude/stats-cache.json` for usage stats since the OAuth API is dead
- **Fuzzy search** — uses skim matcher for instant filtering by session title, project, or model
- **Dynamic context window** — 1M for Opus, 200K for Sonnet, detected from model name
- **Context bar gradient** — 6-stop color gradient: soft blue → blue → soft green → green → yellow → red (80%+)
- **Scrollable chat** — detail view shows wrapped messages with a scrollbar, using ratatui's Paragraph::scroll + Scrollbar widget
- **Auto-refresh** — session list reloads every ~3 seconds to pick up renames and new activity
- **Agent linking** — subagent sessions are linked to parents via directory structure

## User Preferences

- Colors must be readable on both dark mode (navy background with 0.15 transparency) and light mode (warm gray background)
- Labels use `Rgb(140, 140, 170)` (lavender-gray), not DarkGray which maps to invisible navy on the user's theme
- Info panels are more important than the turn-by-turn log — chat at the bottom with scrollbar
- Session list limited to 30 most recent, sorted most recent first
- Navigation: ↑↓ navigate sessions, ←→ cycle info tabs (MCPs/Path/Models/Usage), Enter inspects
- Claude mascot should match the pixel art: 3 crown bumps, dark square eyes, arms, 4 legs
- Preview text capped at 500 chars (not 80) so chat messages aren't truncated
