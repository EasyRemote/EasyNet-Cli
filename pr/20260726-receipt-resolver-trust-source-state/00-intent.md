# Intent

Remove the canonical runtime receipt resolver compatibility path that collapses malformed, missing, and empty realm trust anchors into one generic unavailable message.

Receipt verification is a proof boundary. Trust-source state must remain explicit so product and FFI surfaces can distinguish local signer absence from realm trust corruption.
