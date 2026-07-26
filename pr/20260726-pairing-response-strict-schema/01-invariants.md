# Invariants

1. Pairing validate response must carry explicit device, hub, realm, credential,
   and paired-user facts before credentials are written.
2. Retired aliases are schema errors, not ignored extension fields.
3. Unknown product response fields are version skew and must not be silently
   accepted.
4. Internal tests must not rely on `Default` to fabricate incomplete wire
   contracts.
5. Credentials projection stays one-way: validated product response -> local
   credentials; no SDK abstraction absorbs product pairing fields.
