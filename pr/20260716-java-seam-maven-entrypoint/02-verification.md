# Verification

- `mvn -q -f sdk/java/pom.xml test`
- Surefire report contains `Tests run: 1, Failures: 0, Errors: 0, Skipped: 0`.
- `bash tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`
