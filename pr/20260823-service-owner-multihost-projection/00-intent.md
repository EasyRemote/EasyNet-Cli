# Intent — Service owner multi-host projection

RemoteApp and Pages use Service-owned public abilities as account-scoped callees. A Service may be hosted by more than one paired Device for the same user. The Hub read model must therefore accept multiple live `(service owner, host device)` projections without treating the second host as a same-owner conflict.

This change keeps Service out of Agent directory listings. It only makes Service route resolution use live host Device projections as execution placements.
