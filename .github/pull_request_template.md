<!-- Title: a Conventional Commit, e.g. `feat(agenda): create repeating events`. Target: `main`. -->

## What and why

<!-- What changes, and the problem it solves. Link the issue: Closes #… -->

## How it was tested

<!-- Commands run, what you clicked through, providers used (Microsoft, Google, IMAP, CalDAV…). -->

## Screenshots

<!-- For UI changes: dark and light, and phone width if the layout changes. -->

## Checklist

- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass
- [ ] `pnpm typecheck` and `pnpm build` pass (web), and desktop checks if `desktop/` changed
- [ ] Tests added or updated for the new behaviour
- [ ] Database changes are a new migration (no released migration edited)
- [ ] Guides (`docs/`) and `CHANGELOG.md` (*Unreleased*) updated for user-facing changes
- [ ] No secrets, tokens or personal data in the diff, logs or screenshots
- [ ] New dependencies explained above
