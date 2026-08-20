# Invariants

1. Authority metadata is a generic Runtime Core admission concern.
2. Node does not create canonical authority payloads or signatures.
3. Node does not verify authority cryptography.
4. Exactly one authority metadata family may be attached to an Invocation.
5. Unrelated Invocation metadata is preserved when authority metadata is merged.
6. Authority minting is delegated to an injected transport.
7. No product authentication, session, HTTP, or browser policy is introduced.
8. No non-URA naming or retired input-name compatibility is introduced.
9. Session authority DTOs use generic issuer/subject/audience authority facts,
   not backend/user/session product fields.
