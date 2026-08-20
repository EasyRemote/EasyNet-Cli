# Invariants

- The selector-based Java seam harness remains the single test body.
- Maven test discovery delegates to `main(new String[0])` and does not fork a
  second test matrix.
- The shell seam gate continues to compile with `javac -Xlint:all -Werror` and
  execute the same harness directly.
- No Java SDK public API surface changes in this slice.
