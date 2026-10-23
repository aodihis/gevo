# Gevo v1 — design handoff

Source of truth for the UI of Gevo (egui desktop admin for Typesense). Implements the
requirements in `0002 - V1 requirements`. Each `screens/*.dc.html` file is one screen,
written as HTML with inline styles: read it for layout, copy, spacing and colours, then
rebuild it in egui. Don't embed the HTML. All data in the screens is sample data.

## Navigation (decision for open question 1)
Single window. Left sidebar (`Sidebar.dc.html`, ~232 px wide):
- Connection switcher at the top (status dot, profile name, route). Clicking it opens the profile picker.
- "Server" nav: Collections, API keys, Health. These are disabled when there's no active connection (offline launch).
- A collection list for quick jumps to Documents.
- Bottom: Connections (profile manager) and Settings.

Main area: a 52 px header bar (breadcrumb/title + actions), with content below.

## Screens
| File | Screen | Requirement |
|---|---|---|
| Main.dc.html | Connection profiles, offline state, last-test result per profile | §1 |
| ConnectionEdit.dc.html | Profile form + SSH tunnel + step-by-step test (SSH host → SSH auth → port forward → /health → admin key) | §1 |
| Health.dc.html | /health + /stats.json snapshot, manual refresh, optional 10 s auto-refresh | §5 |
| Settings.dc.html | Theme (system/dark/light) + accent colour | added |
| Collections.dc.html | Collection list: docs, fields, default sort; edit/clone/delete | §2 |
| SchemaEditor.dc.html | Alter schema: existing fields locked, add/drop fields, pending-changes panel → one PATCH | §2 |
| ConfirmDelete.dc.html | Destructive-action modal (type the name to delete a collection) | §2/§3/§4 |
| Documents.dc.html | q / query_by / filter_by / sort_by bar, paginated table, JSON detail panel | §3 |
| Import.dc.html | JSONL import: action (create/upsert/update/emplace), per-row failures with Typesense error text | §3 |
| ApiKeys.dc.html | Key list (prefix only) + create form, with a note that keys can't be edited | §4 |
| KeyCreated.dc.html | One-time key reveal with Copy; Done disabled until "I've stored this key" is ticked | §4 |

## Decisions
- **Schema types (open question 2):** basic types (string, string[], int32, int64, float, bool, geopoint, auto) get form controls. object / nested / float[] embeddings are shown read-only, with editing through a "Raw JSON" mode.
- **Confirmation (open question 3):** always a modal dialog. Collection delete requires typing the collection name. Document delete, key revoke and upsert import use a plain confirm/cancel.
- **Error categories:** every failure is labelled with one of
  `NETWORK` (unreachable), `TUNNEL` (SSH connect/auth/forward), `AUTH` (401/403), `API` (other Typesense error).
  Always show Typesense's `message` verbatim.

## Tokens — dark (default)
| Token | Hex |
|---|---|
| page bg | #121315 |
| sidebar bg | #16171A |
| panel bg | #17181B |
| raised / button | #202226 |
| selected row / active nav | #26282D |
| border | #2C2E33 (inputs #34363C) |
| text | #E7E5E0 |
| text secondary | #B9B7AF |
| text muted | #9A9890 |
| danger fill | #B8402C (text #FFFFFF) |
| danger text | #F59A85 |

Status chips (bg / fg): OK #1C3326/#8FE0B0 · AUTH #3A2C14/#F7C978 · TUNNEL #2D2340/#CDB2F5 · NETWORK #1B2A40/#A9C8F7

## Tokens — light
page #FBFAF8 · sidebar #F3F2EE · panel #FFFFFF · border #DDDBD4 · inputs #D3D1CA ·
active nav #E4E2DB · text #1D1D1B · muted #67655F

Status chips: OK #DDF1E4/#1B6B3F · AUTH #F8E7C6/#7A4B05 · TUNNEL #ECE2FA/#5B3AA8 · NETWORK #DDE8FA/#2552A0

## Accents (dark / light)
Amber #E8A33D / #A8650F (default) · Blue #6EA8FE / #2F64C8 · Green #5FC08A / #23804D · Violet #B394F5 / #6C46C9

Text on the accent fill: #1A1408 in dark, #FFFFFF in light.

## Type & sizing
- IBM Plex Sans (UI) and IBM Plex Mono (data, ids, JSON, endpoints). Bundle both as egui font data.
- Base text 13 px; secondary/labels 12 px; section headings 13–15 px semibold.
- Buttons 32 px high, radius 6. Inputs 32–34 px. Table rows ~34 px. Panels radius 8, 1 px borders, no shadows except on modals.

## egui mapping notes
- Theme: build `egui::Visuals` from the token table. "Match system" follows the OS dark/light setting (`ctx.system_theme()` / eframe's theme follow).
- Sidebar: `SidePanel::left`. Header bar: `TopBottomPanel::top` inside the central area.
- Detail panels (document JSON, pending schema changes, test result): `SidePanel::right` or a right column.
- Modals: `egui::Modal` (egui ≥ 0.30) or a centred `Window` with a dimmed backdrop layer.
- Tables: `egui_extras::TableBuilder`.
