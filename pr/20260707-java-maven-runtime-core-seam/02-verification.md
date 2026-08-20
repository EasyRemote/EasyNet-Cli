# Java Maven Runtime Core Seam Verification

## Commands

```bash
bash tools/scripts/check-java-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Evidence

- Maven packages the dependency-free Java seam jar.
- The direct Java seam test still executes the Runtime Core behavior covered by the Java action-adapter report.
- Static guard rejects legacy address spelling and public protocol-wire symbols in Java package sources.
- Java remains `seam`; provider-backed daemon transports remain unsupported.
