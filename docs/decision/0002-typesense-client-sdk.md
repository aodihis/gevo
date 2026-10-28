# 0002 - Typesense client SDK

Status: accepted

## Context

Phase 2.5 (`docs/product/0003-implementation-plan.md`) needs a
`TypesenseClient` trait implementation that talks to the Typesense HTTP API
(collections, documents, keys, health). Two approaches were available:

- Hand-roll HTTP calls against the Typesense API using `reqwest` directly,
  modeling request/response types ourselves.
- Wrap the official Typesense Rust client: https://github.com/typesense/typesense-rust

## Decision

Use the official **typesense-rust** SDK as the implementation behind
`TypesenseClient`, not hand-rolled `reqwest` calls.

## Consequences

- `TypesenseClient` (and its mock for tests, per `AGENTS.md` §2/§8) wraps
  `typesense-rust` types rather than raw HTTP request/response structs.
- If a bug or gap is found in `typesense-rust` while implementing Phase 2.5,
  stop and report it before writing a workaround. Only file a GitHub issue
  against `typesense/typesense-rust` with explicit go-ahead, since that is a
  public action outside this repo.
- If `typesense-rust` turns out to be unmaintained, missing required
  endpoints, or otherwise unworkable, that is a reason to revisit and
  supersede this decision with a new ADR — not to silently fall back to
  hand-rolled `reqwest` calls.
