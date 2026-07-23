# Intent

Converge device start/readiness on the canonical runtime caller-signer custody model.

The current runtime path advertises `paired_user_runtime_signer` after boot-time provisioning, but the CLI start path treats that capability flag as sufficient proof before publishing the runtime projection. Real product failures show that a daemon can appear Ready while later canonical remote invocation fails because the active User caller signer cannot be loaded from key-service custody.

This iteration removes that weak readiness interpretation. A device runtime may publish/attach only when the active paired User URA can produce a verified domain-separated signature through the same canonical runtime caller signer abstraction used by invocation.

