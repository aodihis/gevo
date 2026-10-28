# AGENTS.md

Guidelines for any AI agent (or human) contributing to Gevo. Read this before
starting work; prefer the rules here over defaults.

## 1. TDD

Write the test first, watch it fail, then write the smallest code that makes
it pass, then refactor. Red → Green → Refactor.

- A new function/method lands with its tests in the same change.
- A bug fix lands with a regression test that fails before the fix.
- No "I'll add tests later." If something is hard to test, that is a design
  signal — fix the design.

## 2. Mocks and stubs in tests

Isolate the unit under test. Collaborators that touch the network, the
filesystem, the clock, OS keychain, SSH, or the Typesense server are mocked
or stubbed. Prefer trait-based abstractions (see §8) that take a mock
implementation in tests and the real one in production.

- Use `mockall` (or hand-written stubs for small surfaces) for trait mocks.
- Integration tests may hit a real Typesense in a container, but keep those
  behind a feature flag / `#[ignore]` so the default `cargo test` run stays
  pure-unit and fast.

## 3. Small chunks, PR per feature

Split work into the smallest unit that is independently valuable and
reviewable.

- One feature = one branch = one PR. Don't bundle unrelated changes.
- Commit messages follow **Conventional Commits**
  (`<type>(<scope>): <subject>`, types: `feat`, `fix`, `chore`, `docs`,
  `refactor`, `test`, `perf`, `ci`). Each commit inside the PR is itself
  coherent.
- Request review from **@pullfrogs** on each PR by @-mentioning them in the
  PR description. Wait for approval (or an explicit "no review needed")
  before merging.
- Merge cleanly — squash or rebase so `main` stays linear and readable.
  Delete the branch after merge.

## 4. Keep it simple

Simple, stupid code beats clever code. Three similar lines beat a premature
abstraction.

- Use proven libraries from crates.io over hand-rolled implementations
  (HTTP: `reqwest`; SSH tunnel: `russh` or `openssh`; keychain: `keyring`;
  logging: `tracing` + `tracing-appender`; tests: `mockall`). Pick the
  mainstream one unless there's a concrete reason not to.
- Don't design for hypothetical future requirements. YAGNI.
- No half-finished implementations — if a path isn't done, don't ship it.
- Before committing, run `/simplify` (or an equivalent reuse/simplification/
  efficiency pass) over the changed code and apply what it finds. Catches
  the premature abstractions and dead weight this section warns against
  before they reach review.

## 5. Avoid unnecessary clones

Cloning is a cost. Pay it only when it makes the code materially simpler or
when ownership actually requires it.

- Prefer `&T` / `&mut T` over `T.clone()` in function signatures.
- For `String`, prefer `&str` in parameters; for `Vec<T>`, prefer `&[T]`.
- `#[derive(Clone)]` on structs only when a caller actually needs owned
  copies.
- In particular, avoid clones inside loops and inside hot paths of the UI
  render loop.
- When a clone is genuinely the right call (sharing with another thread,
  returning owned data across a boundary), that's fine — this rule is
  "don't clone by reflex," not "never clone."

## 6. Unit test every function and method

Every public function and method has at least one unit test covering the
happy path and the obvious failure/edge cases. Private helpers get tested
through the public surface they support unless they warrant direct testing
(complex, independently useful).

- Coverage goal is "every behavior has a test," not a number — don't game a
  percentage.
- Tests live next to the code (`#[cfg(test)] mod tests` at the bottom of
  the module) unless they're integration tests, which go in `tests/`.
- **UI carve-out**: egui render functions
  (`fn ui(&mut self, ui: &mut egui::Ui, …)`, `impl Widget` draw code) are
  exempt. Push logic into testable view-model / state functions and test
  those; thin render glue stays uncovered.

## 7. Short, meaningful comments

Write comments only when the *why* is non-obvious (hidden constraint,
subtle invariant, workaround for a specific bug/upstream issue). Never
narrate *what* the code does — well-named identifiers do that.

- No multi-paragraph docstrings; one short line max.
- No references to the current task, ticket number, author or "added for
  X" — those belong in commit messages / PR descriptions and rot.
- Doc comments on public API items are fine when they add real value to
  the caller.

## 8. Abstraction

Use abstractions to make testing possible and to decouple policy from
mechanism — not to show off.

- External collaborators go behind a `trait` (TypesenseClient,
  CredentialStore, SshTunnel). Production provides the real impl, tests
  provide a mock.
- Introduce a trait when a *second* caller or test variant actually needs
  it. Don't invent abstractions for internal code that has one caller and
  no foreseeable second one — delete them when they appear.

## 9. Logging

Use `tracing` + `tracing-appender` for structured logging. Keep the file
small on disk; keep it off by default is not required but *user-togglable*
is.

- `tracing-subscriber` with an env filter (`RUST_LOG` / explicit app
  setting) so levels are tunable per module.
- An app-level setting (persisted in config, surfaced in the Settings
  screen) toggles file logging on/off. When off, no file writer is
  installed; stdout only.
- When on, use `tracing_appender::rolling::daily` (or `hourly`) to a
  dedicated log directory under the OS data dir. Keep a bounded number of
  rotated files (e.g. last 7 days) — delete older ones at startup.
- Never log secrets: admin API keys, Typesense API keys, SSH passwords,
  passphrases, or the full key value shown on key creation. If a struct
  carries a secret, implement `Debug` manually to redact it.
- Wrap in-memory secrets in `zeroize::Zeroizing<_>` (or `secrecy::Secret`)
  so they're wiped from memory on drop. Applies to admin keys, SSH
  passwords/passphrases, and newly-created API key values held in the
  reveal modal.
- Error logs include the `category` from the PRD (NETWORK / TUNNEL / AUTH
  / API) and the Typesense `message` verbatim where applicable.

## Checklist for every PR

- [ ] Tests written first, all tests pass locally (`cargo test`)
- [ ] `cargo fmt` clean, `cargo clippy -- -D warnings` clean
- [ ] `/simplify` (or equivalent) run over the changed code before committing
- [ ] No unnecessary clones introduced; `&`/`&mut` where possible
- [ ] No secrets written to logs or persisted outside the OS keychain
- [ ] Public API items have at least one unit test
- [ ] Commit messages follow the project convention; PR description
      summarises *why*
- [ ] **@pullfrogs** requested as reviewer; wait for approval before merge
