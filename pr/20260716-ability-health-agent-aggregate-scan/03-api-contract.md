# API Contract

## Public Surface

No public CLI, daemon RPC, or catalog metadata shape changes.

## Internal Contract

`scan()` returns the same `ScanPlan` shape but obtains Agent data from one aggregate snapshot. Health record keys remain canonical ability URAs built with `owner_ability_ura`.

## Error Contract

Aggregate load failures are mapped back to existing health-scan diagnostics:

- Registry source failure reports durable Agent registry scan failure.
- Hosted identity source failure reports hosted-Agent URA index scan failure.

## Tenant Rules

Health records are emitted only for hosted LLM Agent owner URAs present in the aggregate snapshot. Missing or ambiguous hosted LLM identity does not create a health record.
