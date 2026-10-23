# 0002 - V1 requirements

Type: product decision

## Summary

Gevo is a native desktop GUI (Windows, macOS, Linux) for administering a
Typesense server/cluster: connection management, collection schema +
document management, and API key management. No Typesense server-hosting —
Gevo only talks to an existing Typesense instance.

## Out of scope for v1

- Hosting/running Typesense itself.
- Multi-user accounts, roles, or app-level login (single local user, local
  machine).
- Historical metrics/graphs, alerting.
- Typesense cluster administration beyond a single node's `/health` and
  `/stats.json` (no multi-node orchestration).
- Auto-update mechanism (manual installs for v1).

## 1. Connection management

- User can create, edit, delete, and name multiple **connection profiles**.
- Each profile stores:
  - Display name
  - Typesense URL (host, port, protocol)
  - Admin API key
  - Optional SSH tunnel config (see below)
- Profiles persist across app restarts.
- **Credential storage**: admin API key and SSH credentials (password/private
  key passphrase) are stored in the OS-native credential store (Windows
  Credential Manager, macOS Keychain, Linux Secret Service via `keyring`
  crate or equivalent). Non-secret fields (name, url, ssh host/user/port)
  live in a local config file.
- **Keyring unavailable** (headless Linux, no Secret Service running, etc.):
  the app surfaces a clear error on connect/save ("OS keyring not available
  — install/enable a Secret Service provider") and refuses to persist
  secrets. **No plaintext fallback.** A profile without a persisted secret
  can still be used in an ephemeral session where the user re-enters the
  admin key per launch.
- "Test connection" action validates reachability + admin key before saving
  and on-demand from the profile list.
- Switching the active connection is a first-class action (profile picker/
  switcher), not a restart-required setting.
- Connection errors (unreachable, bad key, tunnel failure) must surface a
  clear, specific message (distinguish network error vs auth error vs
  tunnel error).

### SSH tunnel support

- Supported when Typesense is only reachable from behind a bastion/jump
  host.
- Per-profile SSH config: host, port, username, auth method (password,
  private key + optional passphrase), local bind port.
- When enabled, Gevo opens a local SSH port-forward (local port → Typesense
  host:port through the SSH host) and connects to Typesense through it.
- Tunnel lifecycle follows the connection lifecycle: opened on connect,
  closed on disconnect/app exit.
- SSH key files are referenced by path; passphrases/passwords go through the
  same OS-keychain storage as the admin key.

## 2. Collection management (schema)

- List all collections on the active connection with key metadata (name,
  document count, field count).
- Create a new collection: name + field schema builder (field name, type,
  facet/index/optional flags, matching Typesense's schema model).
- View a collection's full schema (read-only detail view).
- Edit a collection's schema where Typesense allows it (adding fields via
  the alter-schema API); clearly disable/explain operations Typesense
  doesn't support (e.g. changing an existing field's type).
- Delete a collection, with a confirmation step (destructive, irreversible).
- Clone/duplicate a collection's schema as a starting point for a new one
  (nice-to-have, not blocking).

## 3. Document management (full CRUD)

- Browse documents within a collection: paginated list/table view using the
  collection's schema to render columns.
- Search/filter documents using Typesense query params (q, filter_by,
  sort_by) exposed through a simple query builder or raw-query input.
- View a single document's full JSON.
- Create a new document via a form generated from the schema, or raw JSON
  input.
- Edit an existing document (same form/JSON options).
- Delete a document, with confirmation.
- Bulk import documents from a JSONL file (map to Typesense's import API),
  showing per-row success/failure results.
- Bulk export a collection's documents to JSONL.
- Destructive actions (delete document, delete collection, overwrite via
  import) require explicit confirmation.

## 4. API key management (full lifecycle)

- List existing API keys: id, prefix, description, actions, collections
  scope, expiry — whatever Typesense's key-list endpoint returns (full key
  value is never available after creation, per Typesense's own behavior).
- Create a new key: description, actions (e.g. `documents:search`,
  `collections:*`), collection scope (specific collections or `*`),
  optional expiry, optional value restriction (scoped search key params).
- On creation, display the full key value once, with a clear one-time
  warning and a copy-to-clipboard action.
- Delete/revoke an existing key, with confirmation.
- No editing of an existing key's permissions (Typesense doesn't support
  this — delete and recreate instead); the UI should make this limitation
  clear rather than silently failing.

## 5. Monitoring (basic)

- Per active connection, a simple health panel showing:
  - Reachability / `/health` endpoint result
  - `/stats.json` summary (e.g. requests/latency snapshot Typesense
    exposes) — single current snapshot, not historical.
- No graphs, no alerting, no polling history in v1. Manual refresh is
  sufficient; a lightweight auto-refresh on an interval is a nice-to-have.

## Non-functional requirements

- **Platforms**: Windows, macOS, Linux — single codebase, native builds for
  each (see [GUI framework ADR](../decision/0001-gui-framework-choice.md),
  egui).
- **Memory**: avoid virtual-DOM-style overhead; this is the reason egui was
  chosen over web-view-based approaches — see
  [GUI framework ADR](../decision/0001-gui-framework-choice.md).
- **Offline behavior**: app launches and lets the user manage saved
  connection profiles even with no active/reachable Typesense server; only
  server-dependent views (collections, documents, keys, health) require a
  live connection.
- **Security**: secrets never written to plain-text config or logs; all
  secret storage goes through the OS keychain as described above.
- **Error handling**: every server-dependent action must distinguish and
  clearly report network errors, auth errors, and Typesense API errors
  (with Typesense's own error message surfaced, not swallowed).
- **No telemetry/analytics** collection in v1.

## Open questions for design

- Navigation model: single window with a connection switcher + sidebar
  (collections/keys/health), vs. per-connection windows/tabs?
- How much of the Typesense schema model (nested fields, embeddings/vector
  fields, geo fields) needs first-class UI in v1 vs. falling back to raw
  JSON schema editing for advanced types?
- Confirmation pattern for destructive actions (inline confirm vs. modal
  dialog) — should be consistent across collection delete, document delete,
  key revoke.
