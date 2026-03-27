---
name: cs-feature
description: Manage feature branches and worktrees for claude-stats development. Use this skill whenever the user wants to start a new feature, create a branch or worktree, work on something new, or merge a feature branch back to main. Triggers on phrases like "new feature", "create a branch", "start working on", "worktree", "merge this back", "finish this feature", "feature branch", or any discussion of branching workflow for claude-stats.
---

# CS Feature

Manage the feature lifecycle for claude-stats: create isolated workspaces (worktrees or branches), validate work, and merge back to main.

This skill has two modes: **create** (start a new feature) and **merge** (finish and merge back to main).

---

## Create Mode

Use this when the user wants to start working on something new.

### 1. Ensure clean state

```
source ~/.cargo/env
git status
```

If there are uncommitted changes, ask the user what to do -- don't silently stash or discard work. Options: commit first (use `/ready-ship`), stash, or discard.

### 2. HARD GATE: Ask worktree or feature branch?

**THIS IS A HARD GATE. You MUST stop here and wait for the user's answer before doing ANYTHING else.**

Use the `AskUserQuestion` tool with these two options:

> **How do you want to isolate this work?**
> - **Worktree** — separate directory, session switches into it so the status line shows which worktree you're in. Ideal for parallel work or keeping context separate.
> - **Feature branch** — switch the current checkout to a new branch. Simpler, single directory.

**Rules:**
- Do NOT enter plan mode until the workspace is created and verified
- Do NOT start exploring code, writing plans, or making edits
- Do NOT proceed to Path A or Path B until the user has answered
- Your response after asking MUST end — no additional tool calls, no planning, no exploration
- Once the user answers, complete ALL workspace setup steps (create, verify, report ready) before any other work begins

---

### Path A: Worktree

#### A1. Pull latest main

```
git checkout main
git pull origin main
```

#### A2. Get feature name

Ask the user for a short name if they haven't provided one. Convention: `fix-<description>` or `feature-<description>` (no slashes — this becomes a directory name).

Good names: `fix-search-scroll`, `feature-export-csv`, `fix-context-bar`

#### A3. Create and enter the worktree

Use the `EnterWorktree` tool with the feature name:

```
EnterWorktree(name: "feature-<name>")
```

This creates the worktree at `.claude/worktrees/feature-<name>/`, creates a new branch `feature-<name>`, and **switches the session's working directory into the worktree** — which updates `workspace.current_dir` so the status line shows `⊔ feature-<name>`.

#### A4. Note the worktree path

After `EnterWorktree`, the absolute worktree path is:

```
/Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name>/
```

**IMPORTANT: The shell cwd still resets between Bash commands.** You must use absolute paths for ALL subsequent operations:

- **Bash commands**: Always prefix with `cd /Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name> &&`
- **Read/Edit/Write tools**: Always use the full absolute path above
- **Glob/Grep tools**: Always set `path` to the absolute worktree path
- **Agent tool prompts**: Always tell agents to work in the absolute worktree path

Store the absolute worktree path and use it everywhere.

#### A5. Verify baseline in the worktree

```
cd /Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name> && source ~/.cargo/env && cargo check
```

If this fails, the issue is in main — flag it before the user builds on a broken foundation.

#### A6. Report ready state

Tell the user:
- The worktree path and branch name
- That the status line now shows `⊔ feature-<name>` confirming the session is in the worktree
- That the baseline compiles cleanly
- That all commands, reads, and edits will target the worktree path
- When done, use `/cs-feature` to merge back

**Resuming in a new session**: If the user closes Claude Code and resumes later, open Claude Code from inside the worktree directory (`/Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name>/`) so the session is scoped to the worktree and the status line reflects it.

---

### Path B: Feature branch

#### B1. Pull latest main

```
git checkout main
git pull origin main
```

#### B2. Create the feature branch

Ask the user for a branch name if they haven't provided one. Convention: `feature/<short-description>`:

```
git checkout -b feature/<name>
```

#### B3. Verify you're on the feature branch

```
git branch --show-current
```

Confirm the output is `feature/<name>`. Do NOT proceed if still on main.

#### B4. Verify clean baseline

```
cargo check
```

If this fails, the issue is in main — flag it before the user starts building on a broken foundation.

#### B5. Report ready state

Tell the user:
- What branch they're on (confirmed via `git branch --show-current`)
- That the baseline compiles cleanly
- They can now start making changes
- When done, use `/cs-feature` again to merge back

---

## Merge Mode

Use this when the user says they're done with a feature and wants to merge back to main. Also use when the user says "finish", "merge", "land this", "done with this feature", or "wrap this up."

**IMPORTANT: The shell cwd resets to the original repo after every command. Always use absolute paths to the worktree directory.**

### 1. Detect context

```
git branch --show-current
git worktree list
```

Confirm we're on a feature branch (not main). Note the worktree path and branch name for later cleanup.

### 2. Commit uncommitted work

Check for uncommitted changes:

```
cd /absolute/path/to/worktree && git status
```

If there are uncommitted changes, stage and commit them:

```
cd /absolute/path/to/worktree && git add -A && git commit -m "feat: <description of changes>"
```

Do NOT proceed to build until all work is committed.

### 3. Lint and build

```
cd /absolute/path/to/worktree && cargo clippy 2>&1
cd /absolute/path/to/worktree && cargo build --release
```

If clippy has warnings, fix them, commit the fix, and rebuild. If the build fails, stop — don't merge broken code into main.

### 4. Install the binary

```
cp /absolute/path/to/worktree/target/release/claude-stats ~/.local/bin/claude-stats
```

Always use the full absolute path. Never rely on cwd.

### 5. Verify the binary works

```
~/.local/bin/claude-stats --help 2>&1 || echo "Binary runs"
```

Also verify the binary contains the expected changes. Do NOT proceed to merge if the binary is stale or broken.

### 6. Review what's changing

```
cd /absolute/path/to/worktree && git log main..HEAD --oneline
cd /absolute/path/to/worktree && git diff main..HEAD --stat
```

Show the user the scope before merging.

### 7. Exit the worktree session (if using EnterWorktree)

If this session was started with `EnterWorktree`, exit it first to return to the main repo context:

```
ExitWorktree(action: "keep")
```

This returns `workspace.current_dir` to the main repo. The worktree directory stays on disk — the branch is merged next.

If the session was started in a new Claude Code window opened inside the worktree directory (resume case), skip this step and proceed using absolute paths to the worktree.

### 8. Merge to main

From the main repo:

```
cd /Users/chrisjones/Documents/Projects/claude-stats && git checkout main
cd /Users/chrisjones/Documents/Projects/claude-stats && git pull origin main
cd /Users/chrisjones/Documents/Projects/claude-stats && git merge feature-<name> --no-ff
```

Use `--no-ff` to preserve the branch history as a merge commit. If there are conflicts, show them to the user and help resolve — don't force through.

### 9. Rebuild and verify from main

```
cd /Users/chrisjones/Documents/Projects/claude-stats && cargo build --release
cp /Users/chrisjones/Documents/Projects/claude-stats/target/release/claude-stats ~/.local/bin/claude-stats
```

Run the binary again to verify. This catches merge issues that might not show up until after the merge.

### 10. Clean up

**If the worktree was created with `EnterWorktree`**, remove it:

```
git worktree remove .claude/worktrees/feature-<name>
git branch -d feature-<name>
```

**If using a manual feature branch**, just delete the branch:

```
git branch -d feature/<name>
```

Verify cleanup:

```
git worktree list
git branch
```

Confirm the worktree/branch is gone and we're on a clean main.

### 11. Hand off to ready-ship

After merging, remind the user: "Feature merged to main. Use `/ready-ship` when you're ready to push to GitHub."

Don't push automatically — that's `/ready-ship`'s job.

---

## Edge Cases

**User is already on a feature branch and says "new feature"**: Ask if they want to finish the current feature first or abandon it.

**User wants to merge but has uncommitted changes**: Commit them first, then proceed with merge. Ask before auto-committing.

**Merge conflicts**: Show the conflicting files, explain the conflict, and help resolve. Never use `--force` or `-X theirs/ours` without explicit user approval.

**User wants to abandon a feature branch**: Confirm, then exit the worktree (`ExitWorktree action: "remove"` if in the same session, or `git worktree remove .claude/worktrees/feature-<name> && git branch -D feature-<name>` if resuming). This is destructive — make sure they mean it.

**User asks "what worktrees do I have?"**: Run `git worktree list` and show them.

**Resuming a worktree in a new session**: Open Claude Code from inside the worktree directory (`.claude/worktrees/feature-<name>/`). The session will be scoped to that directory, `workspace.current_dir` will be the worktree path, and the status line will show `⊔ feature-<name>` automatically.
