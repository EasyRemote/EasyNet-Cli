# Daemon-Native Join Credential Verification

## Intent

Stop `easynet runtime start` from sending federation-native Hub URA join
credentials through the backend HTTP pairing-token verifier.

## Root Abstraction Problem

Token-paired credentials and federation-native join-lineage credentials were
sharing one verifier path. Token-paired credentials are backend product state;
Hub URA join credentials are daemon-native runtime lineage anchored by a join
receipt hash and pinned Hub public key.

## Expected Effect

- Runtime starts can use daemon-native Hub URA join credentials without a
  backend HTTP dependency.
- Token-paired credentials still fail closed through the existing backend
  verifier.
