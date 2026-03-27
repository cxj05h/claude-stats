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
- For worktrees: the shell cwd resets after every Bash command — you MUST use absolute worktree paths for ALL subsequent Bash commands, Read/Edit/Write/Glob/Grep tool calls, and Agent prompts. A plain `cd` does NOT persist.

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

#### A4. Set worktree as working directory

**IMPORTANT: The shell cwd resets to the original repo after every Bash command. A plain `cd` does NOT persist.** You must use absolute paths for ALL subsequent operations in this session:

- **Bash commands**: Always prefix with `cd /Users/chrisjones/Documents/Projects/claude-stats-<name> &&`
- **Read/Edit/Write tools**: Always use `/Users/chrisjones/Documents/Projects/claude-stats-<name>/` as the base path
- **Glob/Grep tools**: Always set `path` to `/Users/chrisjones/Documents/Projects/claude-stats-<name>/`
- **Agent tool prompts**: Always tell agents to work in `/Users/chrisjones/Documents/Projects/claude-stats-<name>/`

Store the absolute worktree path and use it everywhere. Never use relative paths — they resolve to the original repo.

#### A5. Verify baseline in the worktree

```
cd /Users/chrisjones/Documents/Projects/claude-stats-<name> && source ~/.cargo/env && cargo check
```

If this fails, the issue is in main — flag it before the user builds on a broken foundation.

#### A6. Report ready state

Tell the user:
- The absolute worktree path (`/Users/chrisjones/Documents/Projects/claude-stats-<name>/`)
- The branch name (`feature/<name>`)
- That the baseline compiles cleanly
- That all commands, reads, and edits will target the worktree path (not the original repo)
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

Use this when the user says they're done with a feature and wants to merge back to main. Also use when the user says "finish", "merge", "land this", "done with this feature", or "wrap this up."

**IMPORTANT: The shell cwd resets to the original repo after every command. Always use absolute paths to the worktree directory. Never use relative paths like `target/release/` — they'll resolve to the original repo's stale binary.**

### 1. Detect context

```
git branch --show-current
git worktree list
```

Confirm we're on a feature branch (not main). If on main, ask what branch they meant.
If the user is in a worktree directory, note the worktree path for cleanup later.

### 2. Commit uncommitted work

Check for uncommitted changes:

```
cd /absolute/path/to/workspace && git status
```

If there are uncommitted changes, stage and commit them:

```
cd /absolute/path/to/workspace && git add -A && git commit -m "feat: <description of changes>"
```

Do NOT proceed to build until all work is committed. This ensures the merge will include everything.

### 3. Lint and build

```
cd /absolute/path/to/workspace && cargo clippy 2>&1
cd /absolute/path/to/workspace && cargo build --release
```

If clippy has warnings, fix them, commit the fix, and rebuild. If the build fails, stop — don't merge broken code into main.

### 4. Install the binary

```
cp /absolute/path/to/workspace/target/release/claude-stats ~/.local/bin/claude-stats
```

Always use the full absolute path. Never rely on cwd.

### 5. Verify the binary works

Run the installed binary to confirm it launches and shows the new changes:

```
~/.local/bin/claude-stats --help 2>&1 || echo "Binary runs"
```

Also verify the binary contains the expected changes (e.g., `strings` check for new/removed text). Do NOT proceed to merge if the binary is stale or broken.

### 6. Review what's changing

```
cd /absolute/path/to/workspace && git log main..HEAD --oneline
cd /absolute/path/to/workspace && git diff main..HEAD --stat
```

Show the user the scope before merging.

### 7. Merge to main

From the **original repo** (not the worktree):

```
cd /original/repo/path && git checkout main
cd /original/repo/path && git pull origin main
cd /original/repo/path && git merge feature/<name> --no-ff
```

Use `--no-ff` to preserve the branch history as a merge commit. If there are conflicts, show them to the user and help resolve — don't force through.

### 8. Rebuild and verify from main

After merging, rebuild from main to confirm the merged code works:

```
cd /original/repo/path && cargo build --release
cp /original/repo/path/target/release/claude-stats ~/.local/bin/claude-stats
```

Run the binary again to verify. This catches merge issues that might not show up until after the merge.

### 9. Clean up

**If using a worktree**, remove it after merge:

```
git worktree remove ../claude-stats-<name>
git branch -d feature/<name>
```

**If using a feature branch**, just delete the branch:

```
git branch -d feature/<name>
```

Verify cleanup:

```
git worktree list
git branch
```

Confirm the worktree/branch is gone and we're on a clean main.

### 10. Hand off to ready-ship

After merging, remind the user: "Feature merged to main. Use `/ready-ship` when you're ready to push to GitHub."

Don't push automatically — that's `/ready-ship`'s job.

---

## Edge Cases

**User is already on a feature branch and says "new feature"**: Ask if they want to finish the current feature first or abandon it.

**User wants to merge but has uncommitted changes**: Commit them first, then proceed with merge. Ask before auto-committing.

**Merge conflicts**: Show the conflicting files, explain the conflict, and help resolve. Never use `--force` or `-X theirs/ours` without explicit user approval.

**User wants to abandon a feature branch**: Confirm, then `git checkout main && git branch -D feature/<name>`. If it's a worktree, also run `git worktree remove ../claude-stats-<name>`. This is destructive — make sure they mean it.

**User asks "what worktrees do I have?"**: Run `git worktree list` and show them.
