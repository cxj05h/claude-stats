# claude-stats

A native terminal dashboard for Claude Code usage. Browse sessions, inspect token usage, view context breakdowns, and read chat history -- all from a standalone TUI.

![Rust](https://img.shields.io/badge/Rust-ratatui-orange)

## Features

- **Session browser** -- 30 most recent sessions, sorted by activity, with fuzzy search
- **Live detection** -- prominent green indicator with `◉ live` label on the active session
- **Token usage** -- input/output/cache read/cache write per session
- **Context breakdown** -- system prompts, user messages, tool results, assistant output, images
- **Model & effort tracking** -- model timeline and effort level changes within a session
- **MCP tool usage** -- counts per MCP server
- **Scrollable chat history** -- full conversation with markdown rendering, code blocks, diff highlighting
- **In-chat search** -- `/` to search within chat, `n`/`N` for next/prev match, highlighted results
- **Animated mascot** -- pixel art Claude with blinking eyes and wing flap on scroll
- **Agent linking** -- subagent sessions linked to parents via directory structure
- **Weekly usage stats** -- Opus/Sonnet token totals from local stats cache
- **Auto-refresh** -- session list reloads every ~3 seconds
- **Dark & light theme support** -- colors tuned for both terminal backgrounds

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

### Build from source

```bash
git clone https://github.com/cxj05h/claude-stats
cd claude-stats
cargo build --release
cp target/release/claude-stats ~/.local/bin/
```

Requires Rust 1.70+.

## Usage

```bash
claude-stats
```

### Keybindings

| Key | Action |
|-----|--------|
| `Up/Down` | Navigate sessions / scroll chat |
| `Enter` | Open session detail |
| `Left/Right` | Cycle info tabs (list) / prev/next session (detail) |
| `Esc/q` | Back / quit |
| `f` | Toggle fullscreen chat |
| `/` | Search within chat |
| `n/N` | Next/previous search match |
| `c` | Resume session in Claude Code |
| `PgUp/PgDn` | Fast scroll chat |
| `Home/End` | Jump to top/bottom of chat |
| Type anything | Fuzzy search sessions |

## Data Sources

claude-stats reads directly from Claude Code's local files -- no API calls needed:

- `~/.claude/projects/` -- session JSONL files
- `~/.claude/stats-cache.json` -- weekly token statistics
- `~/.claude/stats-config.json` -- plan type configuration
- `~/.claude/settings.json` -- effort level

## Dependencies

- [ratatui](https://github.com/ratatui/ratatui) -- terminal UI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) -- terminal manipulation
- [serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json) -- JSONL parsing
- [chrono](https://github.com/chronotope/chrono) -- timestamp handling
- [dirs](https://github.com/dirs-dev/dirs-rs) -- home directory resolution
- [fuzzy-matcher](https://github.com/lotabout/fuzzy-matcher) -- session search
