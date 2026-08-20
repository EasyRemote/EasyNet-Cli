Goal: add an explicit cross-repository Runtime Events adapter gate without
claiming Runtime Events are cutover-ready.

The gate should prove that the generic Go/Python SDK Runtime Events facades,
Backend SDK event subscription adapter, Backend SDK event stream opener and
EasyRemote product event consumer tests remain coherent. The final live
cross-repository event E2E remains a separate cutover requirement.
