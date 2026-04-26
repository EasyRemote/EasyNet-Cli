//! Agent profile registry — RFC-001 §1 implementation profiles.
//!
//! Per AXON-RFC-001 plan v4.1.2 §A4: "profile" is documentation
//! shorthand for "an Agent advertising the corresponding ability
//! namespace". These are NOT protocol-level types or kind values.
//! They are implementation modules that group ability handlers by
//! the Agent that hosts them.
//!
//! Registered profiles
//! -------------------
//!   device   — fleet.*, observe.*, admin.*, meta.*, schedule.*,
//!              loop.*, discuss.* (host-resident operational abilities)
//!   consent  — consent.* (human-in-the-loop approval flow)
//!   policy   — policy.* (admission policy evaluation)
//!   mcp      — mcp.bridge.* + mcp.client.* (edge MCP adapter, single
//!              Agent owns both inbound and outbound per RFC §1 [P6])
//!   llm      — conversation.*, session.*, meta.* per LLM sub-agent
//!              (claude / codex / etc.)
//!
//! The handlers themselves live in the per-feature files (chat_ability.rs,
//! session_ability.rs, etc.) at the parent agents/ module. The profile
//! files here only declare WHICH abilities each profile owns; the actual
//! `register_*` functions are imported from the feature modules.
//!
//! See:
//!   docs/rfc/AXON-RFC-001-plan-v4.1.2.md §1 — profile catalogue
//!   docs/rfc/AXON-RFC-001-plan-v4.1.2.md §18 — standard ability registry
//!   docs/rfc/AXON-RFC-001-restatement-mapping.md — old → new mapping

pub mod device;
pub mod consent;
pub mod policy;
pub mod mcp;
pub mod llm;
