---
name: cs-debug
description: Debug claude-stats issues by tailing the log file at ~/.claude/stats.log. Use when the user reports a bug, weird behavior, unexpected key action, rendering glitch, or says "debug", "what happened", "check the log", "cs-debug", or mentions something went wrong in claude-stats.
---

# cs-debug — Claude Stats Log Inspector

## Log Location
`~/.claude/stats.log` — auto-rotated daily (truncated on first write of a new day).

## What the Log Contains
- **Startup**: terminal size, session count
- **Every key event**: key code, modifiers (Shift/Ctrl), current mode (List/Detail), active tab, archive state, multi-select count
- **Mode transitions**: List → Detail (with session title), Detail → List
- **Archive operations**: add/remove count, total archived

## Workflow

1. Read the log file:
   ```
   Read ~/.claude/stats.log
   ```

2. Look for the relevant time window — the user will describe when the issue happened.

3. Trace the key sequence to understand what happened:
   - Check which mode the app was in
   - Check if modifiers (Shift) were held
   - Check if a guard condition (tab index, archive state) caused unexpected routing
   - Look for rapid repeated keys (event queue draining)

4. If the log doesn't have enough info, suggest adding more logging to the specific area and rebuilding:
   ```bash
   # In the relevant code path in src/main.rs or src/ui.rs:
   cs_log!("description: relevant_var={}", var);
   cargo build --release && cp target/release/claude-stats ~/.local/bin/claude-stats
   ```

5. After identifying the issue, fix it in the source code.

## Quick Commands
```bash
# Tail live (in another terminal)
tail -f ~/.claude/stats.log

# Last 50 lines
tail -50 ~/.claude/stats.log

# Search for specific key
grep "Char('A')" ~/.claude/stats.log
```
