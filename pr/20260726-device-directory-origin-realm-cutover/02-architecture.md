# Architecture

`federation.discover` is the read-model authority for cross-runtime directory
membership. `src/cli/commands/devices.rs` performs a view projection for the CLI
only. The projection is allowed to map protocol status into display state, but
identity/provenance facts stay canonical.

The retired `tenant_id` field was a product-era alias. Keeping it in the row
made the CLI JSON envelope a second vocabulary authority and undermined the
runtime-wide realm/URA naming cutover.
