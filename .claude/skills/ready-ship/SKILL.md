---
name: ready-ship
description: Ship claude-stats changes to GitHub. Use this skill whenever the user says "ready ship", "push", "ship it", "deploy", "send it", or wants to commit and push their work to the remote repo. Also use when the user asks to update the README or wants to make sure everything is tracked before pushing.
---

# Ready Ship

Ship the current state of claude-stats to GitHub. This skill handles the full commit-and-push pipeline: lint, build, track new files, update the README, commit, and push.

## Workflow

### 1. Validate the build

Before shipping anything, make sure the code is clean:

```
source ~/.cargo/env
cargo clippy 2>&1
cargo build --release
```

If clippy has warnings, fix them before proceeding. If the build fails, stop and report the error -- never push broken code.

### 2. Install the binary

Copy the fresh build so the installed version matches what's being shipped:

```
cp target/release/claude-stats ~/.local/bin/claude-stats
```

### 3. Track new files

Check for untracked files that should be in the repo:

```
git status
```

If there are new source files (`.rs`, `.toml`, `.md`, etc.), stage them. Ignore build artifacts -- the `.gitignore` covers `target/`.

If a file was deleted, make sure it's properly removed from tracking too (`git rm`).

### 4. Review changes and update the README

Look at what changed since the last commit:

```
git diff
git diff --cached
git log --oneline -5
```

Read the current `README.md` and decide:

- **New feature added?** Add it to the Features section. Keep the same bullet style.
- **Keybinding changed?** Update the keybindings table.
- **Dependency added/removed?** Update the Dependencies section.
- **Feature removed or deprecated?** Remove it from the README. Don't leave stale descriptions.
- **Nothing user-facing changed?** Leave the README alone. Internal refactors don't need README updates.

The README should always accurately describe what the app currently does -- no aspirational features, no stale descriptions. When in doubt, read the source to verify.

### 5. Commit

Stage all relevant changes and create a commit. Write a message that describes what changed and why, not just "update files":

```
git add <specific files>
git commit -m "$(cat <<'EOF'
<concise description of changes>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

Prefer specific file adds over `git add .` to avoid accidentally committing sensitive files.

### 6. Push

```
git push origin main
```

If on a feature branch (not main), push that branch instead -- don't push to main. The `/cs-feature` skill handles merging feature branches.

### 7. Confirm

Report back with:
- What was committed (files changed, summary)
- The commit hash
- Confirmation it pushed successfully
- Any README changes made
