# Invariants

1. Cross-device synthetic carrier evidence remains a lower-bound smoke, not
   product-complete RemoteApp evidence.
2. Cross-device RemoteApp product evidence must observe distinct caller and
   provider device URAs.
3. Local-provider-only topology is a failure.
4. Display, window, and application target scenarios must all be represented.
5. Every scenario must bind governed RemoteApp abilities to the selected
   Resource URA and session id.
6. Capture evidence must come from the provider device and bind the selected
   Resource URA.
7. Media evidence must render on the caller side and bind the same provider,
   selected Resource URA, and session id.
8. Input policy must be explicitly observed, even when the session is view-only
   or policy-blocked.
9. Terminal receipts must be visible for teardown.
10. Child verifiers never emit `product_complete_claim=true`; only the aggregate
    product-completion gate may do so.
