# Architecture

`easynet reset` is the local product lifecycle terminal transition. It may
delete EasyNet-Cli daemon state, but it must not change Axon protocol
admission or descriptor resolution rules.

The old shape created pressure to add compatibility fallbacks elsewhere:

1. reset deleted only credentials;
2. stale keyring/read-model/registry state survived;
3. later invocation paths observed mixed old/new state;
4. resolver/signer code appeared broken and invited fallback repair.

The converged shape is:

1. ordinary reset remains credential reset;
2. explicit purge reset removes the local EasyNet state root;
3. subsequent boot starts from a clean canonical runtime state;
4. invocation/resolver/keyring stay fail-closed.
