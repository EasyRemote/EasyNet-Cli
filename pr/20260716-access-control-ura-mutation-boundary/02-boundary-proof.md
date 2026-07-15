Boundary proof
==============

Mutation boundary
-----------------

Grant, list, check, permission-request create, permission-request resolve and
permission-request list normalize identity through URA fields before invoking
runtime abilities. If a caller supplies a scalar `principal_id` without a
canonical `principal_ura`, the request is rejected before provider invocation.

Projection boundary
-------------------

Provider responses may still include historical scalar fields such as
`principal_id` and `owner_user_id`. The SDK keeps those as read-only projection
fields so existing observers can inspect provider output, but the fields do not
feed back into outgoing mutation payloads.
