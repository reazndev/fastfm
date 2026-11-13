# Contributing

The rules for this repository are in [AGENTS.md](AGENTS.md). They were written for coding agents
and apply to people in the same way: size limits, crate boundaries, the Rust and web UI rules,
tests and commit style. [PLAN.md](PLAN.md) explains the design.

The short version: responses must match Last.fm in field names, order, types, error codes and
HTTP status. When Last.fm does something odd, copy it and say so in a comment. New behaviour
comes only as new parameters or new endpoints.

## Checks

Run the checks before every commit. A commit that fails them does not go in.

```sh
scripts/check.sh
```

It runs the file-size check, `cargo fmt --check`, `clippy` with warnings as errors, the Rust
tests, and the web UI's lint, format check, tests and build. Do not silence a check or raise a
limit to get past it; split the code instead. New dependencies must pass `cargo deny check`.

For changes to API responses, also run the conformance harness against the recorded fixtures:

```sh
scripts/conformance.sh --expect conformance/expected.txt
```

It needs `data/conformance.db`, an import of the fixture user; see
[conformance/README.md](conformance/README.md).

## Adding a Last.fm method

1. Create `crates/api/src/methods/<package>/<method>.rs` with a unit struct that implements
   `Method`. `crates/api/src/methods/example/echo.rs` shows the shape, and
   `crates/api/src/methods/shared/` holds what local methods share (paging, list envelopes,
   entity nodes). A handler parses parameters, calls the store, and returns a response tree; it
   never writes XML or JSON itself.
2. Declare it in `crates/api/src/methods/<package>/mod.rs` and add a `.with(...)` line to
   `registry()` in `crates/api/src/methods/mod.rs`. A method that is only forwarded to Last.fm
   needs no file: add it with its cache lifetime to `FORWARDED` in
   `crates/api/src/passthrough/methods.rs`.
3. Record fixtures for it: add its variants to the catalogue in `crates/recorder/src/specs/`
   and run `cargo run -p lfm-record -- --only <method>` with your Last.fm key and user in
   `.env`. Never commit a real key; fixtures store it as `REDACTED`.
4. Run `scripts/conformance.sh --only <method>` until both XML and JSON are clean, or each
   remaining difference has a rule with a reason in `conformance/ignore.toml`. Then update the
   baseline with `scripts/conformance.sh --write-expect conformance/expected.txt`.
5. Update `docs/api.md`, and `README.md` and `PLAN.md` if the change affects them.

Tests never call the real Last.fm. Only the fixture recorder does, and it runs by hand.

## Commits

Use conventional commits, one logical change each, for example
`feat(api): add user.getTopTags`. Keep refactors and behaviour changes in separate commits.
Every bug fix comes with a test that fails without the fix. A new dependency needs a reason in
the commit message.

Schema changes are new numbered files in `migrations/`, listed in `MIGRATIONS` in
`crates/store/src/db.rs`. Never edit a migration that is already committed.
