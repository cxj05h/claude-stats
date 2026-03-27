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
> - **Worktree** — separate directory on disk, work on the feature without leaving your current directory. Ideal for parallel work or keeping context separate.
> - **Feature branch** — switch the current checkout to a new branch. Simpler, single directory.

**Rules:**
- Do NOT enter plan mode until the workspace is created and verified
- Do NOT start exploring code, writing plans, or making edits
- Do NOT proceed to Path A or Path B until the user has answered
- Your response after asking MUST end — no additional tool calls, no planning, no exploration
- Once the user answers, complete ALL workspace setup steps (create, verify, report ready) before any other work begins
- For worktrees: you MUST `cd` into the worktree directory so all subsequent work happens there

---

### Path A: Worktree

#### A1. Pull latest main

```
git checkout main
git pull origin main
```

#### A2. Get feature name

Ask the user for a short name if they haven't provided one. Convention: `feature/<short-description>`.

Good names: `feature/search-filters`, `feature/export-csv`, `feature/fix-scroll-offset`

#### A3. Create the worktree

Place the worktree as a sibling directory to the current repo:

```
git worktree add ../claude-stats-<name> -b feature/<name>
```

This creates `../claude-stats-<name>/` as an isolated checkout on the new branch.

#### A4. cd into the worktree

**You MUST change into the worktree directory so all subsequent commands, edits, and file reads happen there — not in the original repo.**

```
cd ../claude-stats-<name>
```

#### A5. Verify baseline in the worktree

```
cargo check
```

If this fails, the issue is in main — flag it before the user builds on a broken foundation.

#### A6. Report ready state

Tell the user:
- The worktree path (`../claude-stats-<name>/`)
- The branch name (`feature/<name>`)
- That the baseline compiles cleanly
- That this session is now working from the worktree directory
- When done, use `/cs-feature` to merge back

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

Use this when the user says they're done with a feature and wants to merge back to main. Also use when the user says "finish", "merge", "land this", or "done with this feature."

### 1. Detect context

```
git branch --show-current
git worktree list
```

Confirm we're on a feature branch (not main). If on main, ask what branch they meant.
If the user is in a worktree directory, note the worktree path for cleanup later.

### 2. Lint and build

```
cargo clippy 2>&1
cargo build --release
```

If clippy has warnings, fix them. If the build fails, stop — don't merge broken code into main.

### 3. Install and sanity check

```
cp target/release/claude-stats ~/.local/bin/claude-stats
```

### 4. Review what's changing

```
git log main..HEAD --oneline
git diff main..HEAD --stat
```

Show the user the scope before merging.

### 5. Merge to main

```
git checkout main
git pull origin main
git merge feature/<name> --no-ff
```

Use `--no-ff` to preserve the branch history as a merge commit. If there are conflicts, show them to the user and help resolve — don't force through.

### 6. Clean up

**If using a worktree**, remove it after merge:

```
git worktree remove ../claude-stats-<name>
git branch -d feature/<name>
```

**If using a feature branch**, just delete the branch:

```
git branch -d feature/<name>
```

### 7. Hand off to ready-ship

After merging, remind the user: "Feature merged to main. Use `/ready-ship` when you're ready to push to GitHub."

Don't push automatically — that's `/ready-ship`'s job.

---

## Edge Cases

**User is already on a feature branch and says "new feature"**: Ask if they want to finish the current feature first or abandon it.

**User wants to merge but has uncommitted changes**: Commit them first, then proceed with merge. Ask before auto-committing.

**Merge conflicts**: Show the conflicting files, explain the conflict, and help resolve. Never use `--force` or `-X theirs/ours` without explicit user approval.

**User wants to abandon a feature branch**: Confirm, then `git checkout main && git branch -D feature/<name>`. If it's a worktree, also run `git worktree remove ../claude-stats-<name>`. This is destructive — make sure they mean it.

**User asks "what worktrees do I have?"**: Run `git worktree list` and show them.
