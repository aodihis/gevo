# 0001 - GUI framework choice

Status: accepted

## Context

Gevo needs a native GUI framework. The explicit goal is to avoid the memory
overhead associated with virtual-DOM UI models (e.g. React/Electron-style
stacks). Three Rust-native candidates were considered:

- **egui** — immediate-mode GUI, pure Rust, no retained widget tree. Mature,
  large ecosystem, easy to embed. Weaker OS-native look/feel and more
  friction for very complex custom layouts.
- **iced** — Elm-style architecture (retained model, view tree rebuilt on
  update), wgpu-based rendering. Good documentation, actively maintained,
  suited to larger structured apps. Slightly higher memory/CPU overhead than
  egui due to the retained tree.
- **gpui** — Zed's GPU-accelerated native framework, built for
  high-performance, text-heavy UIs (code editors). Excellent raw
  performance but less mature/documented outside of Zed's own codebase.

None of the three use a virtual DOM; all render natively. The differentiator
is memory/CPU profile and maturity for our use case.

## Decision

Use **egui** as the GUI framework for Gevo.

Rationale: immediate-mode rendering with no retained widget tree gives the
lowest baseline memory footprint of the three options, which directly
matches the project's stated priority. egui's ecosystem and documentation
are also the most mature, reducing setup risk.

## Consequences

- UI code will be written in egui's immediate-mode style (UI described each
  frame, no persistent widget tree to manage).
- We give up some native OS look/feel compared to platform-native toolkits.
- If future requirements demand a large, deeply structured UI or
  editor-grade text performance, this decision should be revisited (see
  iced/gpui trade-offs above) and superseded.
