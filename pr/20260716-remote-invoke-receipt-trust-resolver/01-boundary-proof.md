# Boundary Proof

Receipt finalization is an Axon proof over signed checkpoints. EasyNet-Cli owns
the daemon trust source that supplies verifier keys; transport helpers may load
or pass that resolver, but they do not own independent receipt trust policy.

The boundary is:

1. construct the forwarded request and response binding from the signed
   invocation envelope;
2. load the realm trust anchor selected by `EASYNET_REALM_TRUST_PATH` or the
   daemon default trust path;
3. adapt that anchor through `RealmTrustAnchorKeyResolver`;
4. verify admission and terminal receipts before decoding output;
5. fail closed on malformed trust state or missing signer keys.

`LocalKeyServiceReceiptResolver` remains scoped to local daemon client receipt
projection where the signer is expected to be locally provisioned.
