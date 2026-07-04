# Invariants

1. The backend gate is enforcement, not a readiness claim.
   - Passing synthetic self-tests proves the scanner's rules.
   - Running the scanner against a real backend may still fail until backend
     cutover work is complete.

2. Production code is the target.
   - Go test files and generated protobuf files are ignored.
   - Production imports and runtime subprocess markers are checked.

3. The public CLI Go SDK is the allowed runtime boundary.
   - `easynet.run/cli/sdk/go` is allowed.
   - Direct Axon SDK/protobuf and direct daemon transport packages are not.

4. URA terminology remains unchanged.
   - No URI wording is introduced.
