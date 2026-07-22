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
// the URAs used by internal descriptor-bound invocation producers. Schedule
// ticks and loop iterations are not allowed to infer or hardcode runtime
// identity at their call sites.
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

use crate::core::domain::{NodeId, TenantId};

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
    pub fn from_callee_ura(callee_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::core::ura::parse_ura(callee_ura)
            .map_err(|error| anyhow::anyhow!("project runtime session callee URA: {error}"))?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!(
                "runtime session read-model projection requires Device callee URA, got {}",
                parsed.kind
            );
        }
        let device_id = parsed
            .device_id()
            .ok_or_else(|| anyhow::anyhow!("runtime session Device callee omitted device id"))?;
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
    fn projects_session_read_model_from_device_callee_ura() {
        let projection =
            LocalRuntimeSessionProjection::from_callee_ura("easynet:///r/tenant-a/device/device-a")
                .unwrap();

        assert_eq!(projection.tenant().as_str(), "tenant-a");
        assert_eq!(projection.node().as_str(), "device-a");
    }

    #[test]
    fn rejects_non_device_session_read_model_callee() {
        let err = LocalRuntimeSessionProjection::from_callee_ura("easynet:///r/tenant-a/authority")
            .unwrap_err();

        assert!(
            err.to_string().contains("requires Device callee URA"),
            "unexpected error: {err}"
        );
    }
}
