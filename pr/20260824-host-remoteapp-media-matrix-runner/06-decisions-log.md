# Decisions

- Keep fixture commands external because host/network impairment mechanisms are
  deployment-specific, while their lifecycle and evidence boundary are generic.
- Require reset rather than assuming a clean host after a scenario.
- Do not store commands because they may contain credentials or topology.
- Do not let this runner declare product completion; it produces one domain artifact.
