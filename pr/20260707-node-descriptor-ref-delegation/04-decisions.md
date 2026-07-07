# Decisions

1. Node DescriptorRef helpers remain projection-only facade methods over the
   injected Identity transport.
2. Node receipt fetch carriers require `descriptor_ref` from the caller and
   forward it to the Receipt transport without synthesis.
3. Node is declared for `invocation/descriptor_ref_helper_delegation` because
   tests now cover projection delegation, ability derivation delegation,
   receipt fetch forwarding, and missing descriptor rejection.
4. Local daemon/C ABI providers remain out of scope for this seam evidence.
