// EasyNet CLI — Local runtime invocation identity
// =================================================
//
// File: src/daemon/execution/runtime_identity.rs
// Description: Canonical identity projection for daemon-owned LocalRuntime
//              invocation producers.
//
// Protocol Responsibility
// -----------------------
// Own the conversion from daemon runtime state (`realm`, local device node) to
// the URAs used by internal descriptor-bound invocation producers. It keeps
// semantic callees distinct from execution-host Devices.
//
// Implementation Approach
// -----------------------
// A small immutable value object carries the configured realm and local device
// node. It exposes semantically named URA constructors for local callee URAs
// and runtime-resource subjects.
//
// Usage Contract
// --------------
// Construct this object only from daemon config plus paired credentials/control
// discovery. Do not construct it from environment variables or legacy default
// tenant strings.
//
// Architectural Position
// ----------------------
// Execution sub-services own daemon state and enter the Kernel through
// descriptor-bound SDK requests. This object is the shared identity seam that
// keeps those producers on the same canonical runtime model.

use crate::core::domain::{DeferredInvocationAuthority, NodeId, TenantId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRuntimeInvocationIdentity {
    realm: String,
    local_node: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRuntimeSessionProjection {
    tenant: TenantId,
    node: NodeId,
}

impl LocalRuntimeSessionProjection {
    pub fn from_execution_host_ura(execution_host_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::core::ura::parse_ura(execution_host_ura).map_err(|error| {
            anyhow::anyhow!("project runtime session execution-host URA: {error}")
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!(
                "runtime session read-model projection requires Device execution-host URA, got {}",
                parsed.kind
            );
        }
        let device_id = parsed
            .device_id()
            .ok_or_else(|| anyhow::anyhow!("runtime session execution host omitted device id"))?;
        Ok(Self {
            tenant: TenantId::new(parsed.realm.clone()),
            node: NodeId::new(device_id),
        })
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    pub fn node(&self) -> &NodeId {
        &self.node
    }
}

impl LocalRuntimeInvocationIdentity {
    pub fn from_system_agent_callee(callee_ura: &str) -> anyhow::Result<Self> {
        let callee = crate::core::ura::parse_ura(callee_ura)
            .map_err(|error| anyhow::anyhow!("parse automation SystemAgent callee: {error}"))?;
        let Some((device_id, _system_agent_id)) = callee.device_agent_ids() else {
            anyhow::bail!(
                "deferred automation creation requires a device-sponsored SystemAgent callee"
            );
        };
        Self::new(callee.realm.clone(), NodeId::new(device_id))
    }

    pub fn new(realm: impl Into<String>, local_node: NodeId) -> anyhow::Result<Self> {
        let realm = realm.into();
        let realm = realm.trim();
        if realm.is_empty() {
            anyhow::bail!("local runtime invocation identity requires a non-empty realm");
        }
        if local_node.as_str().trim().is_empty() {
            anyhow::bail!("local runtime invocation identity requires a non-empty local node");
        }
        Ok(Self {
            realm: realm.to_string(),
            local_node,
        })
    }

    pub fn realm(&self) -> &str {
        &self.realm
    }

    pub fn local_node(&self) -> &NodeId {
        &self.local_node
    }

    pub fn local_device_ura(&self) -> String {
        self.device_ura_for_node(self.local_node.as_str())
    }

    pub fn device_ura_for_node(&self, node_id: &str) -> String {
        crate::core::ura::device_ura(self.realm(), node_id)
    }

    pub fn resource_subject_ura(&self, resource_name: &str, resource_path: &str) -> String {
        crate::core::ura::resource_dot_ura(self.realm(), resource_name, resource_path)
    }

    /// Resolve a locally hosted User Agent by its persisted placement.
    ///
    /// The returned URA is the invocation callee. The host Device remains a
    /// routing/session fact and is validated here rather than substituted into
    /// the semantic tuple.
    pub fn local_hosted_agent_ura(&self, agent_name: &str) -> anyhow::Result<String> {
        let snapshot =
            crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
                .map_err(|error| anyhow::anyhow!("load hosted Agent placement: {error}"))?;
        let identity = snapshot
            .hosted_agent_identity_by_name(agent_name)
            .map_err(|error| anyhow::anyhow!("resolve hosted Agent {agent_name:?}: {error}"))?
            .ok_or_else(|| {
                anyhow::anyhow!("hosted Agent {agent_name:?} is not registered on this runtime")
            })?;
        let expected_authority = format!("hosted_by:{}", self.local_device_ura());
        if identity.signing_authority != expected_authority {
            anyhow::bail!(
                "hosted Agent {agent_name:?} placement authority {:?} does not match local execution host {:?}",
                identity.signing_authority,
                self.local_device_ura()
            );
        }
        let parsed = crate::core::ura::parse_ura(identity.agent_ura)
            .map_err(|error| anyhow::anyhow!("parse hosted Agent {agent_name:?} URA: {error}"))?;
        if parsed.kind != crate::core::ura::URAKind::Agent || parsed.agent_ids().is_none() {
            anyhow::bail!("hosted Agent {agent_name:?} does not project a User Agent callee");
        }
        if parsed.realm != self.realm {
            anyhow::bail!(
                "hosted Agent {agent_name:?} realm {:?} does not match runtime realm {:?}",
                parsed.realm,
                self.realm
            );
        }
        Ok(identity.agent_ura.to_string())
    }

    pub fn deferred_user_authority(
        &self,
        accountable_user_ura: &str,
        creator_invocation_id: &str,
        controller_callee_ura: &str,
        target_agent_name: &str,
        requested_target_node: &NodeId,
    ) -> anyhow::Result<DeferredInvocationAuthority> {
        let caller = crate::core::ura::parse_ura(accountable_user_ura)
            .map_err(|error| anyhow::anyhow!("parse deferred accountable User: {error}"))?;
        if caller.kind != crate::core::ura::URAKind::User || caller.user_id().is_none() {
            anyhow::bail!(
                "deferred automation requires a canonical User Principal caller, got {accountable_user_ura:?}"
            );
        }
        if caller.realm != self.realm {
            anyhow::bail!(
                "deferred accountable User realm {:?} does not match runtime realm {:?}",
                caller.realm,
                self.realm
            );
        }
        if requested_target_node != &self.local_node {
            anyhow::bail!(
                "deferred local automation target node {:?} does not match controller host {:?}; remote deferred dispatch is not implemented",
                requested_target_node,
                self.local_node
            );
        }
        let creator_invocation_id = creator_invocation_id.trim();
        if creator_invocation_id.is_empty() {
            anyhow::bail!("deferred automation requires its admitted creator invocation id");
        }
        let controller = crate::core::ura::parse_ura(controller_callee_ura)
            .map_err(|error| anyhow::anyhow!("parse deferred controller callee: {error}"))?;
        let Some((controller_device_id, _)) = controller.device_agent_ids() else {
            anyhow::bail!("deferred controller must be a device-sponsored SystemAgent");
        };
        if controller.realm != self.realm || controller_device_id != self.local_node.as_str() {
            anyhow::bail!("deferred controller SystemAgent is not sponsored by this runtime host");
        }
        Ok(DeferredInvocationAuthority {
            accountable_user_ura: accountable_user_ura.to_string(),
            creator_invocation_id: creator_invocation_id.to_string(),
            controller_callee_ura: controller_callee_ura.to_string(),
            target_callee_ura: self.local_hosted_agent_ura(target_agent_name)?,
            execution_host_ura: self.local_device_ura(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_device_and_resource_uras_from_configured_realm() {
        let identity =
            LocalRuntimeInvocationIdentity::new("tenant-a", NodeId::new("node-a")).unwrap();

        assert_eq!(
            identity.local_device_ura(),
            "easynet:///r/tenant-a/device/node-a"
        );
        assert_eq!(
            identity.device_ura_for_node("node-b"),
            "easynet:///r/tenant-a/device/node-b"
        );
        assert_eq!(
            identity.resource_subject_ura("loop.loop-a", "body/1"),
            "easynet:///r/tenant-a/resource/loop.loop-a/body/1"
        );
    }

    #[test]
    fn rejects_empty_realm_before_invocation_production() {
        let err = LocalRuntimeInvocationIdentity::new("  ", NodeId::new("node-a")).unwrap_err();
        assert!(
            err.to_string().contains("non-empty realm"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn projects_session_read_model_from_execution_host_ura() {
        let projection = LocalRuntimeSessionProjection::from_execution_host_ura(
            "easynet:///r/tenant-a/device/device-a",
        )
        .unwrap();

        assert_eq!(projection.tenant().as_str(), "tenant-a");
        assert_eq!(projection.node().as_str(), "device-a");
    }

    #[test]
    fn rejects_non_device_session_execution_host() {
        let err = LocalRuntimeSessionProjection::from_execution_host_ura(
            "easynet:///r/tenant-a/authority",
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("requires Device execution-host URA"),
            "unexpected error: {err}"
        );
    }
}
