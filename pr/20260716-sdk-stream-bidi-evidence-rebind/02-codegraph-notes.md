# CodeGraph Notes

- `codegraph status .`: index up to date with 934 files, 32,472 nodes, 120,726
  edges.
- `codegraph node sdk/go/bidi_test.go`: confirms Go bidi tests own the
  non-terminal cancel and terminal-cancel rejection evidence used by the action
  adapter report.
- `codegraph node sdk/python/tests/test_bidi.py`: confirms Python bidi tests own
  the matching non-terminal cancel and terminal-cancel rejection evidence.
- `codegraph explore "stream bidi cancel conformance action adapter evidence report sdk parity matrix"`:
  identifies the conformance blast radius as SDK report/matrix evidence plus
  stream/bidi cancel surfaces, not daemon runtime behavior.
