# Boundary Proof

## Core trust model

The trust model owns parsing, validation, canonicalization, persistence, and
storage load-state reporting. It does not own daemon first-run policy, CLI
display policy, or receipt resolver fallback policy.

## Daemon boot

Daemon boot owns the first-run policy. Missing trust-anchor storage is accepted
only during initial load and is emitted as an explicit operational event.

## Daemon reload

Reload is a live-state replacement operation. A missing trust-anchor file is an
operator error and must fail closed while preserving the existing cell.

## CLI projection

The CLI may show "no trusted hubs" when the file is missing because it is a read
projection for humans, not the authority path.

## Receipt resolver

Receipt resolver state remains auditable: loaded, empty, missing, and failed
loads are distinguishable.
