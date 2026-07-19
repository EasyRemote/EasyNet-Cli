# Decisions Log

## 2026-07-19T07:02:24Z

- Treat plugin sidecar helpers as EasyNet provider namespace APIs, not
  canonical SDK root APIs.
- Treat unsupported language templates as a capability-matrix issue rather than
  a template-generation convenience gap.

## 2026-07-19T07:08:00Z

- Keep `easynet plugin init --language` limited to languages whose provider
  sidecar helpers are provider-backed or cutover-ready.
- Record Rust, Node, Java, and C/C++ as closed helper seams rather than
  generating templates that would duplicate daemon sidecar frame parsing.
- Promote the template no-naked-frame rule from unit tests into the canonical
  convergence gate.

## 2026-07-19T07:13:30Z

- Full SDK cutover readiness passed after the sidecar helper matrix gate was
  added, including downstream consumer checks and live daemon SDK smokes.
