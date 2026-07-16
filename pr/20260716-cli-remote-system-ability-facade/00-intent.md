# CLI Remote System Ability Facade

## Goal

Move target-owned remote system ability dispatch out of individual CLI command
modules and into one CLI daemon-client facade.

## Root Abstraction Problem

Several CLI commands independently resolve remote device targets, derive
target-owned ability selectors, choose the caller URA, and call the daemon
remote invocation primitive. That makes command modules a second routing facade
beside the daemon-client boundary.

## Expected Effect

- Architecture convergence: command modules map arguments to ability payloads;
  `cli::daemon_client` owns target-owned remote invocation projection.
- Product acceleration: new remote system ability commands get one narrow
  client API instead of repeating routing assembly.
- Cleanliness: the lower `remote_invoke` primitive remains available for
  descriptor-bound origin-proof flows, while simple CLI system ability sugar is
  centralized.
