# Boundary Proof

## SPEC Surface

The daemon SDK requirements define the Compatibility profile file operations as upload/get/delete. The Go SDK canonical method is `CompatibilityClient.GetFile`; Python exposes the same surface as `get_file`.

## Cutover Decision

`RetrieveFile` is removed rather than retained as a public synonym. The backend product caller now consumes `GetFile`, preserving SDK ownership of compatibility-to-invocation projection while removing an obsolete public method name.

## Downstream Boundary

EasyNet backend remains responsible for HTTP/OpenAI response shaping, auth, CORS, and product policy. It no longer requires an SDK legacy method alias to reach daemon-governed file retrieval.
