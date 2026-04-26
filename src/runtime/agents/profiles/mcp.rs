//! mcp profile — RFC-001 §1 [P6].
//!
//! Per restatement-mapping decision P6: a single mcp-profile Agent
//! owns BOTH inbound and outbound MCP — `mcp.bridge.*` (incoming MCP
//! tools/list + tools/call) and `mcp.client.*` (outgoing MCP calls
//! to external servers). They share one Agent identity rather than
//! splitting into two profiles.
//!
//! This is the ONLY place MCP awareness is permitted in the CLI per
//! RFC-001 §A3 (MCP only at edge adapters; everywhere else is
//! Invocation-only). The conformance script enforces this.
//!
//! Owned ability namespaces
//! ------------------------
//!   mcp.bridge.list_tools  (inbound MCP server: tools/list)
//!   mcp.bridge.call_tool   (inbound MCP server: tools/call)
//!   mcp.client.list        (outbound: list configured external MCP servers)
//!   mcp.client.call        (outbound: dispatch to external MCP server)
//!
//! Implementation deferred to P3 (mcp.rs file lands when the actual
//! MCP-in-process server + outbound MCP client are wired up). For
//! P2.3 this is just the profile declaration.

pub const MCP_PROFILE_ABILITY_PREFIXES: &[&str] = &["mcp.bridge.", "mcp.client."];

pub fn owns(ability_name: &str) -> bool {
    MCP_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every mcp.bridge.* + mcp.client.* in the
/// live registry, anchored to the mcp-profile's canonical URA. All
/// SCOPED per §18 — local MCP clients only for bridge.*; the daemon
/// itself + selected internal callers for client.*. P4.7 narrows.
pub fn descriptors_for(
    owner_agent_uri: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    crate::runtime::agents::published_abilities()
        .into_iter()
        .filter(|m| owns(&m.name))
        .map(|m| {
            AbilityDescriptor::new(m.name.clone(), owner_agent_uri, Visibility::Scoped)
                .expect("registry-derived names satisfy descriptor invariants")
                .with_input_schema(m.input_schema.clone())
                .with_source("kernel:built-in")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_both_mcp_namespaces() {
        assert!(owns("mcp.bridge.list_tools"));
        assert!(owns("mcp.bridge.call_tool"));
        assert!(owns("mcp.client.list"));
        assert!(owns("mcp.client.call"));
    }

    #[test]
    fn owns_rejects_other_profiles_and_bare_mcp() {
        assert!(!owns("mcp.evaluate")); // not in either bridge/client subset
        assert!(!owns("fleet.list_abilities"));
        assert!(!owns("consent.subscribe"));
    }
}
