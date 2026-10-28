# 0003 - Implementation plan

Type: product decision

Reference: [v1 requirements](0002-v1-requirements.md), [GUI framework
ADR](../decision/0001-gui-framework-choice.md), contributor rules in
`AGENTS.md`.

## Approach

Build Gevo in thin vertical slices. Each phase ships something runnable
and verifiable. Inside a phase, each feature is its own branch → PR →
review by **@pullfrogs** → clean merge. TDD throughout: write the test
first, then the smallest code that passes it (see `AGENTS.md` §1).

Branch naming: `feat/<slug>`, `fix/<slug>`, `chore/<slug>`.

## Phase 0 — Bootstrap

Goal: a project you can `cargo build` and `cargo test` on all three
platforms, with CI, logging and a frozen copy of the design handoff.

| # | Task | Deliverable |
|---|---|---|
| 0.1 | Scaffold Rust project | `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `.gitignore` |
| 0.2 | Add dependencies | `eframe`, `egui_extras`, `tokio`, `reqwest`, `serde`, `serde_json`, `tracing`, `tracing-subscriber`, `tracing-appender`, `keyring`, `russh` (or `openssh`), `zeroize` (or `secrecy`), `thiserror`, `anyhow`, `mockall` (dev), `tempfile` (dev). Added as each feature PR needs them — not front-loaded. |
| 0.3 | CI | GitHub Actions: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on windows/macos/ubuntu. Separate opt-in / nightly job runs `cargo test -- --ignored` against a Typesense container for the Phase 4+ integration tests. |
| 0.4 | Copy design handoff into `docs/design/` | `DESIGN_HANDOFF.md` + `screens/*.dc.html` committed |
| 0.5 | Logging bootstrap | `tracing-subscriber` + daily rotating file via `tracing-appender`, toggle flag, 7-day retention sweep at startup; secret-redacting `Debug` helper |
| 0.6 | README | How to build, run, test on each OS |

Acceptance: `cargo test` passes with zero tests (or placeholder); CI green
on all three OSes; design docs in-repo; log file rotates and respects the
toggle.

PRs: `chore/scaffold`, `chore/ci`, `chore/design-handoff`, `feat/logging`.

## Phase 1 — Design foundation (theme + shell)

Goal: an empty-looking app that already matches the design tokens, with
sidebar + header shell and theme switching.

| # | Task | PR |
|---|---|---|
| 1.1 | Theme tokens → `egui::Visuals` builder, dark + light + 4 accents | `feat/theme-tokens` |
| 1.2 | Bundle IBM Plex Sans + Mono as egui font data | `feat/fonts` |
| 1.3 | App shell: `SidePanel::left` (232 px) + `TopBottomPanel::top` (52 px header) + `CentralPanel` | `feat/app-shell` |
| 1.4 | `Route` enum + screen dispatch (empty screens for every entry in `DESIGN_HANDOFF.md`) | `feat/routing` |
| 1.5 | `Settings` screen (theme follow/dark/light + accent) wired to live `Visuals` | `feat/settings-screen` |
| 1.6 | Logging on/off toggle in Settings, persisted to config | `feat/log-toggle-ui` |

Tests (per `AGENTS.md` §6):
- `Visuals` builder: given tokens → correct hex per role (dark + light × 4 accents).
- `Route` transitions.
- Config load/save round-trip.

Acceptance: launch app → sidebar + header render in the dark palette;
switch to light / change accent live-updates; Settings persists across
restart.

## Phase 2 — Connection domain (core, no UI)

Pure logic + trait abstractions (per `AGENTS.md` §8). All collaborators
mockable (§2). All functions covered (§6). Avoid clones (§5).

| # | Task | PR |
|---|---|---|
| 2.1 | `ConnectionProfile` struct (non-secret fields: id, name, url, ssh config shape) | `feat/profile-model` |
| 2.2 | `CredentialStore` trait + `KeyringCredentialStore` impl | `feat/credential-store` |
| 2.3 | `ProfileStore` trait + file-backed TOML impl in OS config dir; tests use `tempfile` against the real file-backed impl (filesystem is cheap and faithful to test here) | `feat/profile-store` |
| 2.4 | Error model: `GevoError { Network, Tunnel, Auth, Api(TypesenseMsg) }`, verbatim Typesense `message` | `feat/error-model` |
| 2.5 | `TypesenseClient` trait + implementation wrapping the official `typesense-rust` SDK (see [0002-typesense-client-sdk](../decision/0002-typesense-client-sdk.md)) with methods just for /health, /stats.json, auth check | `feat/typesense-client` |
| 2.6 | `SshTunnel` trait + implementation using `russh` (or `openssh`): open local forward, close on drop | `feat/ssh-tunnel` |
| 2.7 | `ConnectionService` orchestrates: open tunnel (if any) → test /health → auth check, stepwise results | `feat/connection-service` |

Acceptance: `cargo test` covers every public method; mocks exist for all
four traits; no secret appears in debug output or logs.

## Phase 3 — Connection UI

Wire Phase 2 into the shell.

| # | Screen | PR |
|---|---|---|
| 3.1 | Sidebar connection switcher (status dot, profile name, route; opens picker) | `feat/sidebar-switcher` |
| 3.2 | `Main.dc.html` — profile list, offline state, last-test chip per profile | `feat/profiles-main` |
| 3.3 | `ConnectionEdit.dc.html` — profile form + SSH section + step-by-step test (SSH host → SSH auth → port forward → /health → admin key) | `feat/profile-edit` |
| 3.4 | Error chip component (NETWORK / TUNNEL / AUTH / API) | `feat/status-chips` |

Acceptance: user can create → test → save → activate a profile with and
without SSH tunnel; errors show the right category + Typesense message
verbatim.

## Phase 4 — Collections

| # | Task | PR |
|---|---|---|
| 4.1 | `TypesenseClient` extensions: list/get/create/alter/delete collections | `feat/client-collections` |
| 4.2 | `ConfirmDelete.dc.html` modal (type-name-to-delete) — reused by docs/keys | `feat/confirm-modal` |
| 4.3 | `Collections.dc.html` list screen (docs, fields, default sort; edit/clone/delete) | `feat/collections-list` |
| 4.4 | `SchemaEditor.dc.html` — add/drop fields, basic types with form, advanced via Raw JSON, pending-changes panel → one PATCH | `feat/schema-editor` |
| 4.5 | Clone collection as new (nice-to-have) | `feat/collection-clone` |

Acceptance: full schema CRUD against a real Typesense container (behind
`#[ignore]` integration test); mocked unit tests green by default.

## Phase 5 — Documents

| # | Task | PR |
|---|---|---|
| 5.1 | `TypesenseClient` extensions: search, get, create, upsert, update, delete, import, export | `feat/client-documents` |
| 5.2 | `Documents.dc.html` — q / query_by / filter_by / sort_by bar, paginated table using `egui_extras::TableBuilder`, JSON detail panel | `feat/documents-screen` |
| 5.3 | Document create/edit form generated from schema + Raw JSON mode | `feat/document-form` |
| 5.4 | `Import.dc.html` — JSONL import (create/upsert/update/emplace), per-row failures with Typesense error text | `feat/import` |
| 5.5 | Export collection to JSONL file | `feat/export` |

Acceptance: pagination doesn't re-fetch pages already loaded; import
reports per-row results without memory blowup on large files; destructive
actions use the modal from 4.2.

## Phase 6 — API keys

| # | Task | PR |
|---|---|---|
| 6.1 | `TypesenseClient` extensions: list, create, delete keys | `feat/client-keys` |
| 6.2 | `ApiKeys.dc.html` — list (prefix only) + create form + "keys can't be edited" note | `feat/keys-screen` |
| 6.3 | `KeyCreated.dc.html` — one-time reveal with Copy, Done gated on "I've stored this key" checkbox | `feat/key-created-modal` |

Acceptance: full key value is never logged, never persisted beyond the
reveal modal, cleared from memory on close.

## Phase 7 — Health & polish

| # | Task | PR |
|---|---|---|
| 7.1 | `Health.dc.html` — /health + /stats.json snapshot, manual refresh, optional 10 s auto-refresh | `feat/health-screen` |
| 7.2 | Loading/skeleton states across all data views | `chore/loading-states` |
| 7.3 | Keyboard shortcuts (open command palette optional, Esc closes modals, Cmd/Ctrl+K for profile switcher) | `feat/shortcuts` |
| 7.4 | Final a11y + i18n-friendly string extraction pass | `chore/strings` |
| 7.5 | v1 release build + packaging per OS | `chore/release-v1` |

Acceptance: every PRD section (§1–§5) has an end-to-end flow a user can
walk through from launch; no `TODO`/`unimplemented!()` in shipped code.

## Cross-cutting standing rules

- Every PR: tests first (`AGENTS.md` §1), @pullfrogs mentioned,
  `fmt + clippy -D warnings + test` green, no new clones in hot paths,
  no secrets logged.
- Phase can't start until the previous phase's acceptance criteria are
  met.
- Any deviation from a decision in
  [0002-v1-requirements](0002-v1-requirements.md) or
  [0001-gui-framework-choice](../decision/0001-gui-framework-choice.md)
  requires a new ADR under `docs/decision/`.
