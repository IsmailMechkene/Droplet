# Droplet — rules for Claude

## Commits
- Commit at each committable milestone (a stage that compiles and runs).
- Commits are authored by the repository owner only, using the existing git config.
- NEVER add yourself to a commit: no `Co-Authored-By: Claude …` trailer, no "Generated with Claude Code"
  line, no AI attribution of any kind in commit messages, PR bodies, code comments or the README.
- Never change `user.name` / `user.email`, and never use `--author`.
- Do not push unless asked.

## Project constraints (summary — the full brief lives in the README once written)
- Windows only. Tauri 2 + Rust; UI is plain HTML/CSS/vanilla JS (no framework, bundler or npm runtime deps).
- Ask before adding any dependency that is not on the approved list in the README.
- Credentials are read-only and never reach a UI, a log or the disk.
- No cmd/powershell; the only child process the app may start is `agy.exe`.
- `design/Droplet.html` is the source of truth for look, layout, settings and motion.
