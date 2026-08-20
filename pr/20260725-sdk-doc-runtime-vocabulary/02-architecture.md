# Architecture

The SDK documentation is part of the product boundary. If the docs present the
SDK as a daemon or compatibility SDK, downstream products will treat canonical
runtime APIs as product-specific facades. This iteration keeps public imports
stable while aligning the story with the architecture already enforced by code
and conformance gates.
