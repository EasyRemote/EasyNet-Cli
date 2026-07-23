# Boundary Proof

## Lower layer owner

`daemon::identity::self_identity` owns runtime caller signer resolution and signer custody proof. It already differentiates managed User custody from runtime-owner custody, so proof belongs there rather than in CLI start.

## Upper layer consumer

`cli::commands::start` owns product device credentials and daemon start/attach UX. It may derive the active paired User URA from credentials, then require identity-layer proof before writing `runtime.json`.

## Removed weak boundary

`paired_user_runtime_signer` remains a daemon-discovery capability fact, but it is no longer treated as the full custody proof by CLI start. The flag says the daemon reached the provisioning stage; the identity proof says the active caller signer still exists and can sign now.

