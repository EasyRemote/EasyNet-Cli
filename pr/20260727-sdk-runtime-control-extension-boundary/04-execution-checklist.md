1. Remove Go canonical storage of `pagesPort`.
2. Update Go control discovery tests: extension is accepted and ignored; invalid extension does not block canonical attach.
3. Remove Python `_ControlDiscovery.pages_port` and public `RuntimeControlDiscovery.pages_port`.
4. Update Python control/environment tests.
5. Add architecture gate coverage preventing SDK re-export of `pages_port`.
6. Run Go/Python SDK tests and convergence gates.
