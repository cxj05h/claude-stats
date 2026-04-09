---
name: cs-ready-ship
description: Ship claude-stats changes to GitHub. Use this skill whenever the user says "ready ship", "push", "ship it", "deploy", "send it", "release", or wants to commit and push their work to the remote repo. Also use when the user asks to update the README, cut a release, or make sure everything is tracked before pushing.
---

# Ready Ship

Ship the current state of claude-stats to GitHub. This skill handles the full pipeline: clean up git state, lint, build, track new files, update the README, commit, push, and optionally cut a release with cross-platform binaries.

## Workflow

### 0. Hard gate: must be on main

```bash
git branch --show-current
```

If not on `main`, **stop immediately** and tell the user:

> "You're on branch `<branch-name>`. `/ready-ship` only ships from `main`. Run `/cs-feature` to merge your feature branch to main first, then come back here."

Do not proceed. Do not offer to push the feature branch to origin — that's not this workflow's job.

### 1. Clean up git state

**This step ensures a clean foundation before doing anything else.** Main is often dirty from skill edits, external modifications, or leftover worktree branches.

**a. Commit any dirty files on main:**

```bash
git status --short
```

If there are uncommitted changes (modified or untracked source/config files), commit them immediately:

```bash
git add -A
git commit -m "chore: commit pending changes before ship"
```

Do NOT ask the user — these are incidental changes that would otherwise block the workflow. Just commit them.

**b. Prune stale worktrees and branches:**

```bash
git worktree prune
git worktree list
git branch | grep "worktree-"
```

For each `worktree-*` branch that does NOT have a corresponding active worktree directory, delete it:

```bash
git branch -D <stale-branch>
```

Report what was cleaned (e.g., "Cleaned 2 stale branches"). No need to ask — these are always local-only orphans.

**c. Warn about active worktrees:**

If `git worktree list` shows worktrees besides main, warn the user:

> "Active worktree detected: `feature-<name>`. This won't block shipping, but remember to finalize it with `/cs-feature` when done."

Do not block the ship — active worktrees are fine, they're isolated.

### 2. Pull latest

```bash
git pull origin main --no-rebase
```

Use `--no-rebase` so dirty-state pulls create merge commits instead of failing. If the pull fails for other reasons, stop and explain. Never force-push or reset without explicit user approval.

### 3. Validate the build

```bash
source ~/.cargo/env
cargo clippy 2>&1
cargo build --release
```

If clippy has warnings, fix them before proceeding. If the build fails, stop and report the error — never push broken code.

### 4. Install the binary

Copy the fresh build so the installed version matches what's being shipped, then ensure the `cs` symlink points to it:

```bash
cp target/release/claude-stats ~/.local/bin/claude-stats
codesign --sign - ~/.local/bin/claude-stats
ln -sf ~/.local/bin/claude-stats ~/.local/bin/cs
ls -lh ~/.local/bin/claude-stats ~/.local/bin/cs
```

macOS kills unsigned binaries (exit 137) after they are replaced in-place. `codesign --sign -` applies an ad-hoc signature that satisfies Gatekeeper. Always run this after every `cp`. The `ln -sf` keeps the `cs` symlink in sync so both `claude-stats` and `cs` invoke the same binary. Do NOT use `--help` to verify — claude-stats is a TUI and has no `--help` flag; `ls -lh` confirms the file exists and was updated.

### 5. Track new files

Check for untracked files that should be in the repo:

```bash
git status
```

If there are new source files (`.rs`, `.toml`, `.md`, `.yml`, etc.), stage them. Ignore build artifacts — `.gitignore` covers `target/`.

If a file was deleted, make sure it's properly removed from tracking too (`git rm`).

### 6. Review changes and update the README

Look at what changed since the last push:

```bash
git diff
git diff --cached
git log --oneline origin/main..HEAD
```

Read the current `README.md` and decide:

- **New feature added?** Add it to the Features section. Keep the same bullet style.
- **Keybinding changed?** Update the keybindings table.
- **Dependency added/removed?** Update the Dependencies section.
- **Feature removed or deprecated?** Remove it from the README. Don't leave stale descriptions.
- **Nothing user-facing changed?** Leave the README alone. Internal refactors don't need README updates.

The README should always accurately describe what the app currently does — no aspirational features, no stale descriptions. When in doubt, read the source to verify.

### 7. Commit

Stage all relevant changes and create a commit. Write a message that describes what changed and why:

```bash
git add <specific files>
git commit -m "$(cat <<'EOF'
<concise description of changes>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

Prefer specific file adds over `git add .` to avoid accidentally committing sensitive files or worktree directories. If there's nothing new to commit (step 1 already committed everything), skip this step.

### 8. Push

```bash
git push origin main
```

**If push fails:**
- **Authentication error**: Tell the user to check their GitHub credentials. Suggest `gh auth status` to diagnose.
- **Remote has diverged** (`rejected, non-fast-forward`): Do NOT force push. Pull first (`git pull origin main --no-rebase`), resolve any conflicts, then push again.
- **Other errors**: Show the full error output. Don't retry blindly.

### 9. Release (optional)

Ask the user: "Want to cut a release?" If yes (or if they asked for a release upfront):

**a. Bump the version in `Cargo.toml`:**

Determine the next version based on what changed:
- Bug fixes / minor tweaks → patch bump (0.1.0 → 0.1.1)
- New features → minor bump (0.1.0 → 0.2.0)
- Breaking changes → major bump (0.1.0 → 1.0.0)

Ask the user to confirm the version if unsure.

**b. Commit the version bump:**

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version to v<X.Y.Z>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

**c. Tag and push:**

```bash
git tag v<X.Y.Z>
git push origin main --tags
```

Pushing the tag triggers the CI workflow (`.github/workflows/release.yml`) which builds binaries for:
- macOS x86_64
- macOS aarch64 (Apple Silicon)
- Linux x86_64
- Linux aarch64

**d. Monitor the release:**

```bash
gh run list --limit 1
```

Tell the user the CI is building and they can check progress with `gh run watch` or at the GitHub Actions page. Once complete:
- Binaries appear at `https://github.com/cxj05h/claude-stats/releases`
- The `update-homebrew` CI job automatically rewrites `Formula/claude-stats.rb` in `cxj05h/homebrew-tap` with new SHA256s and pushes it

**No manual Homebrew update needed** — the CI handles it end-to-end. After the workflow completes, users can run `brew upgrade claude-stats`.

### 10. Confirm

Report back with:
- What was committed (files changed, summary)
- The commit hash
- Confirmation it pushed successfully
- Any README changes made
- Any git cleanup performed (stale branches, dirty commits)
- If a release was cut: the version tag and CI status
