# Docs

This folder holds the project's living context: why things were decided, and what the product is meant to do.

## Structure

- `decision/` — Architecture Decision Records (ADRs). One file per decision, named `NNNN-meaningful-name.md` (4-digit zero-padded sequence). Each record has a status:
  - `accepted` — currently in effect
  - `superseded` — replaced by a later decision (link to the one that replaces it)
  - `deprecated` — no longer in effect, no replacement
- `product/` — product-facing docs: decisions about what the product does/doesn't do, and free-form notes. Same `NNNN-meaningful-name.md` naming.

## Adding a new decision

1. Copy the next free number in `decision/` (check existing files first).
2. Write the record: context, decision, consequences, status.
3. If it replaces an older one, mark the old one `superseded` and link forward to the new record; mark the new one's "Supersedes" field back to the old one.
