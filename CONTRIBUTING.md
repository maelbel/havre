# Contributing to Havre

Thanks for your interest in Havre. This guide covers how the repository is organised, how branches and commits are
named, and what a pull request needs before it can be merged.

By contributing, you agree that your contribution is licensed under the project's licence,
[AGPL-3.0-or-later](LICENSE).

## Contents

- [Before you start](#before-you-start)
- [Development setup](#development-setup)
- [Branches (Gitflow)](#branches-gitflow)
- [Commit messages (Conventional Commits)](#commit-messages-conventional-commits)
- [Pull requests](#pull-requests)
- [Code guidelines](#code-guidelines)
- [Releases and versions](#releases-and-versions)
- [Reporting bugs and security issues](#reporting-bugs-and-security-issues)

## Before you start

- **Bugs and ideas**: open an issue first (there are templates), so the change can be discussed before you spend time
  on it. Small fixes (typos, an obvious bug) can go straight to a pull request.
- **Security problems**: never in a public issue — see [SECURITY.md](SECURITY.md).
- Be kind: this project follows a [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

You need **Rust** (stable, edition 2024), **Node.js 22** and **pnpm 10**. Havre Desktop also needs the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone https://github.com/maelbel/havre.git
cd havre
cp .env.example .env            # fill it in; .env is never committed

cd web && pnpm install && pnpm build && cd ..
cargo run -p havre-server       # http://localhost:8080
```

For a quick look without a mailbox: `HAVRE_DEMO=1 cargo run -p havre-server`. While working on the UI,
`cd web && pnpm dev` serves it with hot reload on port 3000 and forwards `/api` to the server.

The guides in `docs/` explain how each app works ([user guide](docs/user-guide.md)) and how the code is
laid out ([development](docs/development.md)).

### Checks to run before pushing

```sh
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
cd web && pnpm typecheck && pnpm build
cd desktop/src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # if desktop/ changed
```

The same checks run on every pull request (GitHub Actions, `.github/workflows/`).

## Branches (Gitflow)

| Branch | Purpose | Created from | Merged into |
|--------|---------|--------------|-------------|
| `main` | What is released. Every commit is a version, tagged `vX.Y.Z`. | — | — |
| `develop` | The next release; always builds and passes the checks. | `main` | `main` (through a release branch) |
| `feature/<short-name>` | One feature or change, e.g. `feature/repeating-events`. | `develop` | `develop` |
| `fix/<short-name>` | A bug fix for the next release. | `develop` | `develop` |
| `release/X.Y.Z` | Freezing a release: version numbers, changelog, last fixes. | `develop` | `main` **and** `develop` |
| `hotfix/X.Y.Z` | An urgent fix for the released version. | `main` | `main` **and** `develop` |

- Branch names are lower case, words separated by `-`.
- Nobody pushes directly to `main` or `develop`: changes arrive through pull requests.
- Keep a branch up to date by rebasing it on `develop` (`git pull --rebase origin develop`) rather than merging
  `develop` into it.
- Delete a branch once it is merged.

## Commit messages (Conventional Commits)

Commits follow [Conventional Commits 1.0](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <summary in the imperative, lower case, no final period>

<body: what changed and why, wrapped at 72 characters>

<footer: BREAKING CHANGE: …, Refs: #123>
```

**Types**

| Type | For |
|------|-----|
| `feat` | A new feature for the user |
| `fix` | A bug fix |
| `perf` | Faster or lighter, same behaviour |
| `refactor` | Code restructured, same behaviour |
| `test` | Tests only |
| `docs` | Documentation only |
| `style` | Formatting only (no code change) |
| `build` | Dependencies, build tooling, Docker |
| `ci` | GitHub Actions |
| `chore` | Anything else (repository housekeeping) |

**Scopes** (optional, one per commit): `core`, `server`, `cli`, `web`, `desktop`, `mail`, `agenda`, `people`,
`tasks`, `notes`, `files`, `ai`, `mcp`, `auth`, `backup`, `docker`, `deps`.

**Examples**

```
feat(agenda): create repeating events
fix(mail): keep the selection after archiving
build(web): move the UI to Nuxt UI 4
docs: explain the backup restore steps
feat(core)!: store times as UTC seconds

BREAKING CHANGE: the database needs migration 0018.
```

- One logical change per commit; a commit should build and pass the tests.
- Mark breaking changes with `!` after the type or scope **and** a `BREAKING CHANGE:` footer.
- Reference issues in the footer: `Refs: #42`, or `Closes: #42` to close one.

## Pull requests

1. Open the pull request against **`develop`** (or `main` for a `hotfix/` or `release/` branch).
2. Give it a Conventional Commit title (it becomes the squash commit): `feat(tasks): add sub-steps`.
3. Fill in the template: what changes, why, how it was tested, screenshots for UI changes (dark and light, and phone
   width when the layout changes).
4. Make sure the checks pass. A reviewer approves before merging; pull requests are **squash-merged** into
   `develop`, release and hotfix branches are merged with a merge commit.

A pull request is ready when:

- the checks pass (`fmt`, `clippy` without warnings, tests, `typecheck`, `build`);
- new behaviour has tests next to the code (`#[cfg(test)]`);
- database changes are a **new** migration in `crates/core/migrations/` (released migrations are never edited);
- user-facing changes are described in the guides (`docs/`), and in `CHANGELOG.md` under *Unreleased*;
- no secret, password, token or personal data is in the diff.

## Code guidelines

- **Rust**: logic, SQL and protocols live in `havre-core`; the server only parses requests, calls the core and maps
  errors. One error type (`havre_core::Error`); messages shown to users are full sentences they can act on.
  `rustfmt.toml` sets the width (110). Prefer simple, readable code with a short comment explaining *why*.
- **SQL**: runtime-checked `sqlx::query` (building must not need a database).
- **Times**: Unix seconds in UTC everywhere; all-day items at UTC midnight with an exclusive end.
- **Secrets**: passwords, refresh tokens and API keys are encrypted at rest; session and API tokens are stored hashed.
  Never log them and never send them to the browser.
- **Mail is untrusted**: HTML is sanitised and sandboxed; the assistant only proposes changes, the user confirms.
- **Web**: controls come from [Nuxt UI](https://ui.nuxt.com) themed with Havre's tokens
  (`web/app/assets/css/main.css`, `web/app/app.config.ts`); don't hand-build buttons, fields, menus or dialogs.
  Icons are in `web/icons.ts`. The UI must not load anything from other servers (no remote icons or fonts).
  Every button has an accessible name.
- **Dependencies**: say why in the pull request, prefer what is already used, and pin versions through the lockfiles
  (`Cargo.lock`, `pnpm-lock.yaml`).

## Releases and versions

Havre uses [Semantic Versioning](https://semver.org/) (`MAJOR.MINOR.PATCH`; before 1.0, minor versions may break
things).

1. `git switch develop && git switch -c release/X.Y.Z`
2. Set the version in `Cargo.toml` (`[workspace.package]`), `desktop/src-tauri/Cargo.toml` and
   `desktop/src-tauri/tauri.conf.json`; move *Unreleased* in `CHANGELOG.md` to the new version with today's date.
3. Pull request into `main`; once merged, tag it: `git tag -a vX.Y.Z -m "Havre X.Y.Z"` and push the tag.
4. Merge `main` back into `develop`.

Havre Desktop releases are tagged `desktop-vX.Y.Z`; the *Desktop* workflow builds the installers and drafts the
GitHub release.

## Reporting bugs and security issues

- Bugs: open an issue with the *Bug report* template.
- Vulnerabilities: follow [SECURITY.md](SECURITY.md) (private report, not an issue).
