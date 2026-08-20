# Architecture

The SDK parity matrix owns language-facade capability visibility. Companion
lifecycle is represented as a runtime control-plane capability because the
facades wrap daemon control APIs and DTOs; they do not own platform supervisor
semantics or product policy.

The matrix, shared case, validator, and language expectations move together so
there is one source of truth for capability count and provider-backed evidence.
