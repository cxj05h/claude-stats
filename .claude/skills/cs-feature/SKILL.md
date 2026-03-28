---
name: cs-feature
description: Manage feature branches and worktrees for claude-stats development. Use this skill whenever the user wants to start a new feature, create a branch or worktree, work on something new, or merge/finalize a feature branch back to main. Triggers on phrases like "new feature", "create a branch", "start working on", "worktree", "merge this back", "finish this feature", "finalize this feature", "finalize", "feature branch", or any discussion of branching workflow for claude-stats.
---

# CS Feature

Manage the feature lifecycle for claude-stats: create isolated workspaces (worktrees or branches), validate work, and merge back to main.

This skill has three modes: **create** (start a new feature), **merge** (finish and merge back to main), and **cleanup** (remove stale worktrees/branches).

---

## Create Mode

Use this when the user wants to start working on something new.

### 0. Pre-flight: gitignore safety check

Before creating any worktree, verify `.claude/worktrees/` is in `.gitignore`:

```bash
grep -q '.claude/worktrees' .gitignore
```

If not found, add it and commit immediately:

```bash
echo '.claude/worktrees/' >> .gitignore
git add .gitignore
git commit -m "chore: ignore worktree directories"
```

This prevents worktree contents from ever being accidentally committed.

### 1. Ensure clean state

```bash
source ~/.cargo/env
git status
git branch --show-current
```

**If already on a feature branch**: Ask the user — finish/merge the current feature first, or start a second parallel feature in a new worktree? Never silently abandon existing work.

**If there are uncommitted changes**: Auto-stash them with a descriptive message:

```bash
git stash push -m "cs-feature: auto-stash before creating worktree"
```

These will be popped back after the worktree is created. Never ask the user about stashing — it's internal housekeeping.

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
- Once the user answers, complete ALL workspace setup steps before any other work begins

---

### Path A: Worktree

#### A1. Pull latest main (handle dirty state)

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && git stash push -m "cs-feature: auto-stash before pull" 2>/dev/null; git checkout main 2>/dev/null; git pull origin main; git stash pop 2>/dev/null
```

**Key principle: never let dirty state block a pull.** Auto-stash, pull, pop. If the stash pop conflicts, the stash is preserved — warn the user but continue.

#### A2. Get feature name

Ask for a short name if not already provided. Convention: `fix-<description>` or `feature-<description>`.

**Rules for names:**
- Use dashes only — no slashes (this becomes both a branch name AND a directory name)
- Keep it short and descriptive
- Good: `fix-search-scroll`, `feature-export-csv`, `fix-context-bar`
- Bad: `feature/search` (slashes not allowed as directory names), `f` (too vague)

#### A3. Check for conflicts before creating

```bash
git branch | grep "feature-<name>"
ls .claude/worktrees/ 2>/dev/null
```

If a branch or directory with that name already exists, ask the user:
- **Resume** — enter the existing worktree and continue
- **Replace** — remove the old one and start fresh (confirm before deleting)
- **Rename** — pick a different name

#### A4. Create and enter the worktree

Use the `EnterWorktree` tool:

```
EnterWorktree(name: "feature-<name>")
```

This creates the worktree at `.claude/worktrees/feature-<name>/`, creates branch `feature-<name>`, and switches the session's working directory into the worktree — updating `workspace.current_dir` so the status line shows `⊔ feature-<name>`.

#### A5. Note the worktree path

After `EnterWorktree`, the absolute path is:

```
/Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name>/
```

**CRITICAL: The shell cwd resets between every Bash command.** Always use absolute paths:

- **Bash commands**: Prefix with `cd /Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name> &&`
- **Read/Edit/Write tools**: Use the full absolute path
- **Glob/Grep tools**: Set `path` to the absolute worktree path
- **Agent tool prompts**: Tell agents to work in the absolute worktree path

#### A6. Verify baseline in the worktree

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats/.claude/worktrees/feature-<name> && source ~/.cargo/env && cargo check
```

If this fails, the issue is in main — flag it before building on a broken foundation. Don't proceed until baseline is clean.

#### A7. Report ready state

Tell the user:
- The worktree path and branch name
- That the status line now shows `⊔ feature-<name>` confirming the session is in the worktree
- That the baseline compiles cleanly
- That all commands, reads, and edits will target the worktree path
- When done, type `/cs-feature` to merge back

**Resuming in a new session**: Open Claude Code from inside the worktree directory (`.claude/worktrees/feature-<name>/`) so the session is scoped to that path automatically.

---

### Path B: Feature branch

#### B1. Pull latest main

```bash
git stash push -m "cs-feature: auto-stash before pull" 2>/dev/null; git checkout main 2>/dev/null; git pull origin main; git stash pop 2>/dev/null
```

#### B2. Create the feature branch

Ask for a name if not provided. Convention: `feature/<short-description>` (slashes are fine here — this doesn't become a directory name):

```bash
git checkout -b feature/<name>
```

#### B3. Verify you're on the feature branch

```bash
git branch --show-current
```

Confirm the output matches `feature/<name>`. Do NOT proceed if still on main.

#### B4. Verify clean baseline

```bash
source ~/.cargo/env && cargo check
```

If this fails, the issue is in main — flag it before the user starts building on a broken foundation.

#### B5. Report ready state

Tell the user:
- What branch they're on (confirmed)
- That the baseline compiles cleanly
- They can now start making changes
- When done, type `/cs-feature` to merge back

---

## Merge Mode (Finalize)

Use this when the user is done with a feature and wants to merge back to main. Trigger phrases: "finalize", "finish", "merge", "land this", "done with this feature", "wrap this up."

**This mode commits, merges, cleans up, and rebuilds the binary so the installed version matches main. It does NOT push — shipping to GitHub is a separate step (`/ready-ship`).**

**CRITICAL: The shell cwd resets after every command. Always use absolute paths.**

### 1. Detect context

```bash
git branch --show-current
git worktree list
```

Determine the worktree path and branch name. If already on main with no feature branch, tell the user there's nothing to merge.

### 2. Commit uncommitted work in the worktree

```bash
cd /absolute/path/to/worktree && git status
```

If there are uncommitted changes, show them and ask the user for a commit message (don't invent one):

```bash
cd /absolute/path/to/worktree && git add -A && git commit -m "<user-provided message>"
```

Do NOT proceed until all work is committed.

### 3. Review what's changing

```bash
cd /absolute/path/to/worktree && git log main..HEAD --oneline
cd /absolute/path/to/worktree && git diff main..HEAD --stat
```

Show the user the scope before merging. If the diff is larger than expected, pause and confirm.

### 4. Handle dirty main

**Main will often be dirty.** Skill files, README, or other files get modified while working in worktrees. This is normal and must be handled seamlessly.

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && git status --short
```

**If main has uncommitted changes:**

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && git add -A && git commit -m "chore: commit pending changes on main before merge"
```

Commit them with a generic message. Do NOT ask the user — these are incidental changes (skill edits, external modifications) that would otherwise block the merge. Always commit, never stash (stashes get lost).

**Then sync main with remote:**

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && git pull origin main --no-rebase
```

If pull fails due to divergence, use `--no-rebase` to create a merge commit rather than risking rebase conflicts.

### 5. Exit the worktree session (if using EnterWorktree)

If this session was started with `EnterWorktree`, exit it first:

```
ExitWorktree(action: "keep")
```

This returns `workspace.current_dir` to the main repo. The worktree directory stays on disk — the branch is merged next.

If the session was resumed by opening Claude Code inside the worktree directory, skip this step.

### 6. Merge to main

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && git checkout main
cd /Users/chrisjones/Documents/Projects/claude-stats && git merge worktree-feature-<name> --no-ff -m "Merge worktree-feature-<name>: <brief description>"
```

Use `--no-ff` to preserve the branch history as a merge commit.

**If there are merge conflicts**: Show the conflicting files, explain what's conflicting and why, and help resolve. Never use `--force` or `-X theirs/ours` without explicit user approval.

### 7. Clean up THIS worktree and branch

**Always use `-D` (force delete), not `-d`**, for worktree branches. These branches are never pushed to origin, so git's `-d` always fails with "not fully merged" — even when the commit is safely on local main. `-D` skips the remote tracking check. It's safe because we verified the merge in step 6.

```bash
# Remove the worktree directory:
git worktree remove .claude/worktrees/feature-<name> 2>/dev/null

# Delete the branch:
git branch -D worktree-feature-<name>
```

### 8. Clean up ALL stale worktrees and branches

After the merge, automatically scan for and clean orphaned worktree branches:

```bash
# Find branches that start with "worktree-" but have no active worktree
git branch | grep "worktree-"
git worktree list
```

For each `worktree-*` branch that does NOT have a corresponding active worktree directory:
- It's an orphan from a previous session that wasn't cleaned up
- Delete it silently: `git branch -D <branch-name>`
- No need to ask — these are always local-only branches with no upstream

Also prune any stale worktree metadata:

```bash
git worktree prune
```

Report what was cleaned up (e.g., "Cleaned up 3 stale branches: worktree-fix-search-scroll, worktree-markdown-chat, worktree-feature-session-branch").

### 9. Rebuild and install

The merge may have changed code (conflict resolution, rebase). Always rebuild from main so the installed binary matches what was just merged:

```bash
cd /Users/chrisjones/Documents/Projects/claude-stats && source ~/.cargo/env && cargo clippy 2>&1
cd /Users/chrisjones/Documents/Projects/claude-stats && cargo build --release
```

If clippy has warnings, fix them. If the build fails, stop and fix — never leave a broken binary installed after a merge.

Then install and codesign (macOS kills unsigned replaced binaries):

```bash
cp target/release/claude-stats ~/.local/bin/claude-stats
codesign --sign - ~/.local/bin/claude-stats
ln -sf ~/.local/bin/claude-stats ~/.local/bin/cs
ls -lh ~/.local/bin/claude-stats
```

### 10. Final verification

```bash
git worktree list
git branch
git status
git log --oneline -5
```

Confirm:
- Only expected worktrees remain (just main, or main + any actively in-use worktrees)
- No stale `worktree-*` branches
- Clean working tree
- Merge commit is at HEAD

### 11. Done

Tell the user: "Feature merged, rebuilt, and installed. Use `/ready-ship` when you're ready to push to GitHub."

Do NOT push — that's `/ready-ship`'s job.

---

## Cleanup Mode

Use this when the user asks to clean up worktrees, or when stale branches are detected.

Trigger phrases: "clean up worktrees", "remove stale branches", "git is messy", "what worktrees do I have?"

### 1. Survey the state

```bash
git worktree list
git branch
git stash list
```

### 2. Identify orphans

Compare the `worktree-*` branches against active worktree directories. Any branch without a matching directory is an orphan.

### 3. Clean up

For each orphan:

```bash
git worktree prune
git branch -D <orphan-branch>
```

For stale stashes (older than 1 week with "cs-feature: auto-stash" prefix):

```bash
git stash list | grep "cs-feature: auto-stash"
```

Show them to the user and offer to drop them.

### 4. Report

Show what was cleaned and the final state.

---

## Edge Cases

**Main is dirty when creating a worktree**: Auto-stash, create worktree, pop stash. The worktree gets a clean copy of main; the stash preserves the user's pending changes on main.

**Main is dirty when merging**: Auto-commit with generic message. These are always incidental changes (skill files, external edits) that would otherwise block the workflow. Never stash during merge — stashes get lost.

**Main has diverged from remote during merge**: Use `git pull --no-rebase` to create a merge commit. Never force-push or reset.

**`git pull` fails with "unstaged changes"**: This is the most common annoyance. Always stash-pull-pop or commit first. The skill handles this automatically so the user never sees it.

**Already on a feature branch when starting a new one**: Ask to finish current feature first or work in parallel (new worktree). Never silently switch branches.

**Worktree or branch name already exists**: Ask to resume, replace (confirm before deleting), or rename. Never silently overwrite.

**Merge conflicts**: Show conflicting files, explain what's conflicting and why, help resolve. Never use `--force` or `-X` without explicit user approval.

**Build fails during merge**: Stop. Help fix the build first, then re-attempt. Never merge broken code.

**User wants to abandon a feature**: Confirm explicitly — this is destructive. Then:
- Same session: `ExitWorktree(action: "remove")`
- Different session or plain branch: `git worktree remove .claude/worktrees/feature-<name> && git branch -D worktree-feature-<name>`

**`git branch -d` fails with "not fully merged"**: This is expected for worktree branches. Use `git branch -D` — the branch is merged, git just can't verify it against a remote tracking branch that doesn't exist.

**Resuming a worktree in a new session**: Open Claude Code from inside `.claude/worktrees/feature-<name>/`. The session will scope to that directory automatically.

**`.claude/worktrees/` not in `.gitignore`**: Fix it before creating the worktree (Step 0). Otherwise worktree contents can be accidentally committed.
