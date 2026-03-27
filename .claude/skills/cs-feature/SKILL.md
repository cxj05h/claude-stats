---
name: cs-feature
description: Manage feature branches for claude-stats development. Use this skill whenever the user wants to start a new feature, create a branch, work on something new, or merge a feature branch back to main. Triggers on phrases like "new feature", "create a branch", "start working on", "merge this back", "finish this feature", "feature branch", or any discussion of branching workflow for claude-stats.
---

# CS Feature

Manage the feature branch lifecycle for claude-stats: create branches, validate work, and merge back to main.

This skill has two modes based on what the user needs: **create** (start a new feature) and **merge** (finish and merge back to main).

---

## Create Mode

Use this when the user wants to start working on something new.

### 1. Ensure clean state

```
source ~/.cargo/env
git status
```

If there are uncommitted changes, ask the user what to do -- don't silently stash or discard work. Options: commit first (use `/ready-ship`), stash, or discard.

### 2. Pull latest main

```
git checkout main
git pull origin main
```

### 3. Create the feature branch

Ask the user for a branch name if they haven't provided one. Use the convention `feature/<short-description>`:

```
git checkout -b feature/<name>
```

Good names: `feature/search-filters`, `feature/export-csv`, `feature/fix-scroll-offset`

### 4. Verify clean baseline

Run a quick build check to confirm main is in a good state before starting work:

```
cargo check
```

If this fails, the issue is in main -- flag it before the user starts building on a broken foundation.

### 5. Report ready state

Tell the user:
- What branch they're on
- That the baseline compiles cleanly
- They can now start making changes
- When done, use `/cs-feature` again to merge back

---

## Merge Mode

Use this when the user says they're done with a feature and wants to merge back to main. Also use when the user says "finish", "merge", "land this", or "done with this feature."

### 1. Validate the feature branch

```
source ~/.cargo/env
git branch --show-current
```

Confirm we're on a feature branch (not main). If on main, ask what branch they meant.

### 2. Lint and build

```
cargo clippy 2>&1
cargo build --release
```

If clippy has warnings, fix them. If the build fails, stop -- don't merge broken code into main.

### 3. Run a quick sanity check

Install and launch briefly to verify no runtime crash:

```
cp target/release/claude-stats ~/.local/bin/claude-stats
```

### 4. Review what's changing

Show the user a summary of everything on this branch vs main:

```
git log main..HEAD --oneline
git diff main..HEAD --stat
```

This helps them confirm the scope before merging.

### 5. Merge to main

```
git checkout main
git pull origin main
git merge feature/<name> --no-ff
```

Use `--no-ff` to preserve the branch history as a merge commit. If there are conflicts, show them to the user and help resolve -- don't force through.

### 6. Clean up

After successful merge, delete the feature branch:

```
git branch -d feature/<name>
```

### 7. Hand off to ready-ship

After merging, remind the user: "Feature merged to main. Use `/ready-ship` when you're ready to push to GitHub."

Don't push automatically -- that's `/ready-ship`'s job. This keeps the workflow modular: the user can review the merge on main before shipping.

---

## Edge Cases

**User is already on a feature branch and says "new feature"**: Ask if they want to finish the current feature first or abandon it.

**User wants to merge but has uncommitted changes**: Commit them first, then proceed with merge. Ask before auto-committing.

**Merge conflicts**: Show the conflicting files, explain the conflict, and help resolve. Never use `--force` or `-X theirs/ours` without explicit user approval.

**User wants to abandon a feature branch**: Confirm, then `git checkout main && git branch -D feature/<name>`. This is destructive -- make sure they mean it.
