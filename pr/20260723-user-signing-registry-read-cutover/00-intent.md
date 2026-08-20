# Intent

Move the user signing identity registry read off generic local invocation.

`identity.list_user_pubkeys` is a read projection over daemon-owned identity
trust state. The reconciliation flow already models key creation separately
from trust registration, but its read side still used `invoke_local_ability`.
That kept one daemon-self shortcut inside a signer custody lifecycle path.

This slice routes only the read projection through
`LocalRuntimeStateReadIssuer`; `identity.register_pubkey` remains an explicit
system-root mutation with a descriptor-bound subject.
