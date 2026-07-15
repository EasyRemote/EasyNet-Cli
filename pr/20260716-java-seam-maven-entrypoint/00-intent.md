# Intent

Make the Java runtime-core seam executable through Maven's default test path.

The Java SDK seam already has a dependency-free selector harness. This slice
adds the conventional `test*` entrypoint so `mvn test` executes that same
harness instead of relying only on the repository shell gate.
