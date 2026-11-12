# Agent guide

A self-hosted mirror of the Last.fm API. Read `PLAN.md` before starting work: it holds the
architecture, the schema, and the milestones.

## Before every commit

Run `scripts/check.sh`. It runs the file-size check, `cargo fmt`, `clippy` with warnings as
errors, the tests, and the web checks. A commit that fails it does not go in.

Never silence a check to get past it. `#[allow(...)]` needs a `reason = "..."` and is a last
resort. Do not raise a threshold in `clippy.toml`, `Cargo.toml`, or `scripts/check-file-size.sh`.
If a limit blocks you, split the code.

## Size limits

| Thing | Limit |
|---|---|
| Rust source file | 400 lines |
| TypeScript file | 300 lines |
| HTML or stylesheet | 200 lines |
| Test file | 600 lines |
| Function | 60 lines |
| Function parameters | 5 |
| Nesting depth | 5 |
| Line width | 100 columns |

## Structure

- Crates depend in one direction: `server` → `api` → `store`, `upstream` → `core`. `core` has no
  I/O. `store` is the only crate that contains SQL. `upstream` is the only crate that calls
  Last.fm.
- One Last.fm method per file under `crates/api/src/methods/<package>/<method>.rs`. A handler
  parses parameters, calls the store or the upstream client, and returns a response tree. It
  does not write XML or JSON itself.
- Shared behaviour goes in one place: paging, period ranges, image URLs, and error responses each
  have a single implementation that every method uses.
- SQL schema changes are numbered files in `migrations/`. Never edit a migration that is
  already committed.

## Rust rules

- No `unsafe`. No `unwrap`, `expect`, `panic!`, or indexing with `[]` outside tests. Return a
  `Result`.
- Libraries define error enums with `thiserror`. Only `server` may use `anyhow`.
- No blocking calls on the async runtime. SQLite work goes through the store's thread pool.
- Never hold a lock across an `.await`.
- Log with `tracing`. No `println!`. Never log API keys, secrets, session keys, or passwords.
- New dependencies need a reason in the commit message. Prefer the standard library and crates
  already in the workspace. `cargo deny check` must pass.
- Public items get a one-line doc comment. Other comments explain why, never what.

## Web UI rules

- Angular with standalone components, signals, and `OnPush` change detection. TypeScript in
  strict mode. No `any`.
- When the Angular app is created in `web/`, add ESLint (`angular-eslint`) with `max-lines` 300,
  `max-lines-per-function` 60, `complexity` 10, and `no-explicit-any`, plus Prettier at 100
  columns. Add `lint`, `format:check`, `test`, and `build` scripts, which `scripts/check.sh`
  calls.
- Components only display and emit. HTTP calls live in services.

## Tests

- Every Last.fm method has a conformance test against recorded fixtures in both XML and JSON.
  A method is not done until its diff is clean or each difference is listed in the ignore file
  with a reason.
- Every bug fix comes with a test that fails without the fix.
- Tests never call the real Last.fm. Only the fixture recorder does, and it runs by hand.
- Fixtures are recorded against the Last.fm user named in `.env`. Never commit a real API key,
  secret, session key, or password. Fixtures store the key as `REDACTED`.

## Commits

- Conventional commits: `feat(api): add user.getTopArtists`. One logical change per commit.
- Keep refactors and behaviour changes in separate commits.
- Update `README.md` and `PLAN.md` in the same commit when behaviour or structure changes.

## Compatibility is the product

Responses must match Last.fm in field names, order, types, error codes, and HTTP status. When
Last.fm's behaviour looks wrong, copy it and note it in a comment. Extensions are new parameters
or new endpoints only. They never change what an existing Last.fm request returns.
