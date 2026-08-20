# Java Maven Runtime Core Seam Plan

## Goal

Convert the Java Runtime Core seam into a Maven-packaged JVM facade while keeping its capability state at `seam`.

## Scope

- Add Maven package metadata for the dependency-free `run.easynet.daemon` Java seam.
- Keep direct seam tests as the behavioral gate for typed errors, feature discovery, complete Invocation draft construction, injected runtime transport, and bounded stream/bidi state.
- Update the Java seam guard to validate Maven packaging plus the existing seam behavior.
- Update SDK status documentation so Java package metadata is no longer listed as missing.

## Non-Scope

- No JNI, C ABI, or daemon transport provider.
- No profile clients beyond the Runtime Core seam.
- No provider-backed transport report.
- No product cutover claim.
