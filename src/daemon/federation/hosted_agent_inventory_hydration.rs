//! Hub boot proof for durable hosted-Agent inventory.
//!
//! Durable inventory is necessary but not sufficient to expose an Agent in the
//! realm directory. Every active row must also prove that its hosting Device is
//! owned by the User encoded in the canonical Agent URA. A crash may leave the
//! inventory commit durable before the derived Agent owner binding is written;
//! this plan identifies exactly those safe repairs while rejecting conflicts.

use crate::daemon::persistence::federation_revoke::{
    DurableSigningAuthority, HostedAgentInventoryRecord, InventoryLifecycle,
};
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedPrincipalOwner};

#[derive(Debug)]
pub(crate) struct HostedAgentInventoryHydrationPlan {
    inventory: Vec<HostedAgentInventoryRecord>,
    missing_owner_bindings: Vec<TrustedPrincipalOwner>,
}

impl HostedAgentInventoryHydrationPlan {
    pub(crate) fn prove(
        hub_realm: &str,
        inventory: Vec<HostedAgentInventoryRecord>,
        trust_anchor: &RealmTrustAnchor,
        repaired_at_unix_ms: u64,
    ) -> anyhow::Result<Self> {
        let mut missing_owner_bindings = Vec::new();
        for record in &inventory {
            let expected = prove_inventory_owner(hub_realm, record, trust_anchor)?;
            match trust_anchor.lookup_principal_owner(&record.agent_ura) {
                Some(existing)
                    if existing.owner_user_id == expected.owner_user_id
                        && existing.owner_ura == expected.owner_ura => {}
                Some(_) => anyhow::bail!(
                    "hosted Agent inventory owner binding conflicts with the durable host proof for `{}`",
                    record.agent_ura
                ),
                None => missing_owner_bindings.push(TrustedPrincipalOwner {
                    added_at_unix_ms: repaired_at_unix_ms,
                    ..expected
                }),
            }
        }
        Ok(Self {
            inventory,
            missing_owner_bindings,
        })
    }

    pub(crate) fn missing_owner_bindings(&self) -> &[TrustedPrincipalOwner] {
        &self.missing_owner_bindings
    }

    pub(crate) fn into_inventory(self) -> anyhow::Result<Vec<HostedAgentInventoryRecord>> {
        if !self.missing_owner_bindings.is_empty() {
            anyhow::bail!(
                "hosted Agent inventory hydration remains incomplete after owner-binding repair"
            );
        }
        Ok(self.inventory)
    }
}

fn prove_inventory_owner(
    hub_realm: &str,
    record: &HostedAgentInventoryRecord,
    trust_anchor: &RealmTrustAnchor,
) -> anyhow::Result<TrustedPrincipalOwner> {
    if record.lifecycle != InventoryLifecycle::Active {
        anyhow::bail!(
            "hosted Agent inventory hydration received a non-active row for `{}`",
            record.agent_ura
        );
    }
    let agent = crate::core::ura::parse_ura(&record.agent_ura)
        .map_err(|error| anyhow::anyhow!("hosted Agent inventory URA is invalid: {error}"))?;
    let (owner_user_id, agent_id) = agent.agent_ids().ok_or_else(|| {
        anyhow::anyhow!(
            "hosted Agent inventory requires a canonical user-owned Agent URA: `{}`",
            record.agent_ura
        )
    })?;
    let canonical_agent_ura = crate::core::ura::agent_ura(&agent.realm, owner_user_id, agent_id);
    if agent.realm != hub_realm || record.agent_ura != canonical_agent_ura {
        anyhow::bail!(
            "hosted Agent inventory row is outside the Hub realm or is not canonical: `{}`",
            record.agent_ura
        );
    }

    let host_ura = match &record.signing_authority {
        DurableSigningAuthority::HostedBy { host_ura } => host_ura,
        DurableSigningAuthority::SelfSigned => anyhow::bail!(
            "hosted Agent inventory row lacks Device signing custody: `{}`",
            record.agent_ura
        ),
    };
    let host = crate::core::ura::parse_ura(host_ura)
        .map_err(|error| anyhow::anyhow!("host Device inventory URA is invalid: {error}"))?;
    let host_device_id = host.device_id().ok_or_else(|| {
        anyhow::anyhow!("hosted Agent inventory signing authority is not a Device: `{host_ura}`")
    })?;
    let canonical_host_ura = crate::core::ura::device_ura(&host.realm, host_device_id);
    if host.realm != hub_realm || host.realm != agent.realm || host_ura != &canonical_host_ura {
        anyhow::bail!(
            "hosted Agent inventory host is outside the Agent realm or is not canonical: `{host_ura}`"
        );
    }
    if record
        .host_node_id
        .as_deref()
        .is_some_and(|node_id| node_id != host_device_id)
    {
        anyhow::bail!(
            "hosted Agent inventory host node contradicts its Device URA for `{}`",
            record.agent_ura
        );
    }

    let host_owner = trust_anchor
        .lookup_principal_owner(host_ura)
        .ok_or_else(|| {
            anyhow::anyhow!(
            "host Device `{host_ura}` has no authoritative owner binding; refusing to hydrate `{}`",
            record.agent_ura
        )
        })?;
    let expected_owner_ura = crate::core::ura::user_ura(hub_realm, owner_user_id);
    if host_owner.owner_user_id != owner_user_id || host_owner.owner_ura != expected_owner_ura {
        anyhow::bail!(
            "host Device `{host_ura}` is not owned by Agent URA User `{owner_user_id}`; refusing to hydrate `{}`",
            record.agent_ura
        );
    }

    Ok(TrustedPrincipalOwner {
        principal_ura: record.agent_ura.clone(),
        owner_user_id: host_owner.owner_user_id.clone(),
        owner_ura: host_owner.owner_ura.clone(),
        added_at_unix_ms: host_owner.added_at_unix_ms,
    })
}
