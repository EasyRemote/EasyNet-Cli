# Intent

Converge the bundled remote-desktop plugin abilities onto the governed v2
AbilityDescriptor contract.

Remote desktop is a session-oriented plugin surface with RPC, stream and bidi
abilities. Its descriptor files must declare those modes and receipt semantics
directly so daemon publication, admission and MCP projection do not infer
control-plane facts from legacy schema defaults.
