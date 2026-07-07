# Java Async Runtime Seam Verification

## Commands

```bash
bash tools/scripts/check-java-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Evidence

- Java seam tests cover `CompletableFuture` invocation, observable future cancellation, and iterator support on stream/bidi handles.
- Maven packaging still succeeds with dependency-free Java sources.
- Java remains a Runtime Core seam and does not claim provider-backed transport support.
