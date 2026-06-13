// EasyNet CLI — `meta.teach` / `meta.acquire` / `meta.forget`
// =============================================================
//
// File: src/runtime/agents/teach_ability.rs
// Description: GET route B (seven-axes T3.3, spec §2.5) — capability
//              transfer as the ONTOLOGY defines it: `learn` is the
//              reflexive `meta.acquire` (ONTOLOGY_AGENT_ABILITY:220),
//              `forget` its inverse, and `teach` the cross-owner half
//              that makes either possible. The initiative is always
//              the owner's: no grant in the teach directory means
//              `allow_transferred_code = false` (the InstallPolicy
//              default, capability.proto:235-239) and `meta.acquire`
//              refuses.
//
// v1 scope (spec D6 ruling): same-device, manifest-only. A learn
// copies the taught manifest into the learner agent's workspace —
// the learner's hosted URA (RFC-005 §1.4 mint) then makes the copy a
// NEW ability under the learner's own identity: discover projects
// it, `exactly one owner` holds for both copies, two receipt chains
// account separately. Cross-device transfer rides PayloadTransfer in
// a follow-up; `--with-assets` likewise.
//
// Execution posture: the learn response DECLARES
// `execution_mode = "sandbox_first"` (from the grant; default
// capability.proto:238). Enforcing it inside the shell executor is
// the executor milestone — declared here, never faked.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::persistence::teach_grants::{self, LearnedRecord, TeachGrant, EXECUTION_MODE_DEFAULT};
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::ura::AbilitySelector;

pub const TEACH: &str = "meta.teach";
pub const ACQUIRE: &str = "meta.acquire";
pub const FORGET: &str = "meta.forget";

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_envelope_and_owner(TEACH, OwnerKind::Device, Arc::new(teach_handler));
    reg.register_rpc_with_owner(ACQUIRE, OwnerKind::Device, Arc::new(acquire_handler));
    reg.register_rpc_with_owner(FORGET, OwnerKind::Device, Arc::new(forget_handler));
}

/// Where an ability lives on this device: its owning agent and the
/// flat-layout manifest path.
struct AbilityHome {
    owner_agent: String,
    manifest_path: PathBuf,
}

struct OwnerAuthority {
    owner_ura: String,
    granted_by: String,
}

/// Resolve an owner-local registry name (`<agent>.<ability>`) to the
/// owning agent's on-disk manifest. Precise refusals at every step —
/// a transfer surface must never guess.
fn resolve_owner_manifest(registry_name: &str) -> anyhow::Result<AbilityHome> {
    let Some((agent, public_name)) = registry_name.split_once('.') else {
        anyhow::bail!(
            "ability must use the owner-local `<agent>.<name>` form; got {registry_name:?}"
        );
    };
    let agents = crate::registry::agents::load_agents()?;
    let Some(entry) = agents.agents.get(agent) else {
        anyhow::bail!("no agent {agent:?} on this device (see `easynet agent list`)");
    };
    let Some(root) = entry.root_path.as_ref() else {
        anyhow::bail!("agent {agent:?} has no workspace root; cannot locate its abilities");
    };
    let manifest_path = root
        .join("abilities")
        .join(format!("{public_name}.ability.toml"));
    if !manifest_path.exists() {
        anyhow::bail!(
            "agent {agent:?} publishes no ability {public_name:?} \
             (expected manifest at {})",
            manifest_path.display()
        );
    }
    Ok(AbilityHome {
        owner_agent: agent.to_string(),
        manifest_path,
    })
}

fn required_str<'a>(args: &'a Value, field: &str, surface: &str) -> anyhow::Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{surface} requires `{field}`"))
}

/// Validate that the current invocation caller may confer an
/// ability owned by `owner_agent`.
///
/// Clean v1 rule: the caller must be either the owner Agent URA or
/// the host device named by that owner's persisted signing authority.
/// This keeps local operator UX working without allowing an arbitrary
/// admitted caller to write grants for someone else's ability.
fn require_owner_authority(
    env: &EnvelopeContext,
    owner_agent: &str,
) -> anyhow::Result<OwnerAuthority> {
    let caller = env
        .caller
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{TEACH} requires an invocation caller"))?;
    let local = crate::persistence::local_agents::load()?;
    let Some(owner_entry) = local
        .hosted_agents
        .iter()
        .find(|entry| entry.profile == "llm" && entry.name == owner_agent)
    else {
        anyhow::bail!(
            "owner agent {owner_agent:?} has no minted local Agent URA; \
             teach grants must bind to a persisted owner identity"
        );
    };

    if caller == owner_entry.agent_ura {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: caller.to_string(),
        });
    }

    let expected_host = format!("hosted_by:{caller}");
    if owner_entry.signing_authority == expected_host {
        return Ok(OwnerAuthority {
            owner_ura: owner_entry.agent_ura.clone(),
            granted_by: caller.to_string(),
        });
    }

    anyhow::bail!(
        "{TEACH} caller {caller:?} cannot teach abilities owned by \
         {owner_agent:?}; expected owner {} or its host device authority",
        owner_entry.agent_ura
    );
}

/// `meta.teach { ability, learner_ura }` — the owner confers
/// learnability of one ability to ONE learner. Idempotent per
/// (ability, learner): re-teaching refreshes the grant.
fn teach_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    let ability = required_str(&args, "ability", TEACH)?;
    let learner_ura = required_str(&args, "learner_ura", TEACH)?;
    let parsed = crate::ura::parse_ura(learner_ura)
        .map_err(|e| anyhow::anyhow!("invalid learner_ura {learner_ura:?}: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!("learner_ura must be an Agent URA — abilities are taught to agents");
    }

    let home = resolve_owner_manifest(ability)?;
    let authority = require_owner_authority(&env, &home.owner_agent)?;
    let mut directory = teach_grants::load()?;
    directory
        .grants
        .retain(|g| !(g.ability == ability && g.learner_ura == learner_ura));
    directory.grants.push(TeachGrant {
        ability: ability.to_string(),
        owner_agent: home.owner_agent.clone(),
        learner_ura: learner_ura.to_string(),
        execution_mode: EXECUTION_MODE_DEFAULT.to_string(),
        granted_at: chrono::Local::now().to_rfc3339(),
    });
    teach_grants::save(&directory)?;

    Ok(json!({
        "taught": ability,
        "owner_agent": home.owner_agent,
        "owner_ura": authority.owner_ura,
        "granted_by": authority.granted_by,
        "learner_ura": learner_ura,
        "execution_mode": EXECUTION_MODE_DEFAULT,
    }))
}

/// `meta.acquire { ability_ura, learner }` — the learner (a local
/// agent) acquires a taught ability and becomes the owner of its own
/// copy, under its own URA.
fn acquire_handler(args: Value) -> anyhow::Result<Value> {
    let ability_ura = required_str(&args, "ability_ura", ACQUIRE)?;
    let learner = required_str(&args, "learner", ACQUIRE)?;

    let selector = AbilitySelector::parse(ability_ura)?;
    let registry_name = selector.local_registry_ability();
    let public_name = selector.public_name();

    let agents = crate::registry::agents::load_agents()?;
    let Some(learner_entry) = agents.agents.get(learner) else {
        anyhow::bail!("no agent {learner:?} on this device to learn into");
    };
    let local = crate::persistence::local_agents::load()?;
    let Some(learner_ura) =
        crate::persistence::local_agents::lookup_hosted_ura(&local, "llm", learner)
    else {
        anyhow::bail!(
            "agent {learner:?} has no minted URA (RFC-005 §1.4); \
             it cannot own abilities yet"
        );
    };

    let mut directory = teach_grants::load()?;
    let Some(grant) = directory.grant_for(registry_name, &learner_ura).cloned() else {
        anyhow::bail!(
            "not teachable (allow_transferred_code=false): the owner has not \
             taught {registry_name:?} to {learner_ura}"
        );
    };

    let owner_home = resolve_owner_manifest(registry_name)?;
    let Some(learner_root) = learner_entry.root_path.as_ref() else {
        anyhow::bail!("agent {learner:?} has no workspace root to learn into");
    };
    let dest_dir = learner_root.join("abilities");
    let dest = dest_dir.join(format!("{public_name}.ability.toml"));
    if dest.exists() {
        anyhow::bail!(
            "agent {learner:?} already has an ability named {public_name:?}; \
             forget it first or rename — learning never overwrites"
        );
    }
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dest_dir.display()))?;
    std::fs::copy(&owner_home.manifest_path, &dest).map_err(|e| {
        anyhow::anyhow!(
            "copy manifest {} → {}: {e}",
            owner_home.manifest_path.display(),
            dest.display()
        )
    })?;

    directory.learned.push(LearnedRecord {
        ability_name: public_name.to_string(),
        learner_agent: learner.to_string(),
        learned_from: ability_ura.to_string(),
        learned_at: chrono::Local::now().to_rfc3339(),
    });
    teach_grants::save(&directory)?;

    let new_ura = crate::ura::owner_ability_ura(&learner_ura, public_name)
        .ok_or_else(|| anyhow::anyhow!("could not mint the learner's ability URA"))?;
    Ok(json!({
        "acquired": public_name,
        "new_ura": new_ura,
        "learned_from": ability_ura,
        "execution_mode": grant.execution_mode,
    }))
}

/// `meta.forget { ability, agent }` — drop a LEARNED ability. The
/// learned ledger is the authority: a native ability never matches
/// it, so forget can never silently delete what an agent authored.
fn forget_handler(args: Value) -> anyhow::Result<Value> {
    let ability = required_str(&args, "ability", FORGET)?;
    let agent = required_str(&args, "agent", FORGET)?;

    let mut directory = teach_grants::load()?;
    let Some(idx) = directory.learned_by(agent, ability) else {
        anyhow::bail!(
            "agent {agent:?} never learned {ability:?} — only learned abilities \
             can be forgotten (native abilities are removed by their author, \
             not by forget)"
        );
    };

    let agents = crate::registry::agents::load_agents()?;
    let manifest = agents
        .agents
        .get(agent)
        .and_then(|e| e.root_path.as_ref())
        .map(|root| {
            root.join("abilities")
                .join(format!("{ability}.ability.toml"))
        });
    if let Some(path) = manifest.filter(|p| p.exists()) {
        std::fs::remove_file(&path)
            .map_err(|e| anyhow::anyhow!("remove {}: {e}", path.display()))?;
    }
    let record = directory.learned.remove(idx);
    teach_grants::save(&directory)?;

    Ok(json!({
        "forgotten": ability,
        "agent": agent,
        "had_learned_from": record.learned_from,
    }))
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;
    use crate::registry::agents::{AgentRegistry, AgentType};

    /// Two agents on disk — mentor publishes `quote`, apprentice
    /// publishes nothing — both with minted URAs, all through the
    /// product files.
    fn seed() -> (String, String, String) {
        let home = std::env::var("HOME").expect("HomeGuard sets HOME");
        let mut registry = AgentRegistry::default();
        for name in ["mentor", "apprentice"] {
            let root = std::path::Path::new(&home).join(format!("agents/{name}"));
            std::fs::create_dir_all(root.join("abilities")).expect("agent root");
            std::fs::write(
                root.join("agent.toml"),
                format!("name = \"{name}\"\nruntime = \"claude-code\"\n"),
            )
            .expect("agent.toml");
            let mut entry = crate::registry::agents::AgentEntry::new(AgentType::ClaudeCode, None);
            entry.root_path = Some(root);
            registry.agents.insert(name.to_string(), entry);
        }
        crate::registry::agents::save_agents(&registry).expect("save agents");

        std::fs::write(
            std::path::Path::new(&home).join("agents/mentor/abilities/quote.ability.toml"),
            "name = \"quote\"\ndescription = \"emit a quotable line\"\n\n[input_schema]\ntype = \"object\"\n",
        )
        .expect("mentor manifest");

        let mut local = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: crate::ura::device_ura("localhost", "dev-1"),
            ..Default::default()
        };
        let mentor_ura = crate::ura::agent_ura("localhost", "dev", "mentor");
        let apprentice_ura = crate::ura::agent_ura("localhost", "dev", "apprentice");
        crate::persistence::local_agents::upsert_hosted_agent(
            &mut local,
            "llm",
            "mentor",
            &mentor_ura,
        );
        crate::persistence::local_agents::upsert_hosted_agent(
            &mut local,
            "llm",
            "apprentice",
            &apprentice_ura,
        );
        crate::persistence::local_agents::save(&local).expect("save local agents");

        let taught_ura =
            crate::ura::owner_ability_ura(&mentor_ura, "quote").expect("mentor ability URA");
        (taught_ura, apprentice_ura, mentor_ura)
    }

    fn caller_env(caller: impl Into<String>) -> EnvelopeContext {
        EnvelopeContext {
            caller: Some(caller.into()),
            ..Default::default()
        }
    }

    #[test]
    fn acquire_without_grant_is_the_default_refusal() {
        let _g = HomeGuard::new();
        let (taught_ura, _, _) = seed();
        let err = acquire_handler(json!({ "ability_ura": taught_ura, "learner": "apprentice" }))
            .expect_err("no grant must refuse");
        assert!(
            format!("{err}").contains("allow_transferred_code=false"),
            "the refusal must name the gate: {err}"
        );
    }

    #[test]
    fn teach_then_acquire_mints_the_learner_copy() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();

        let teach = teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("owner teaches");
        assert_eq!(teach["owner_ura"], teach["granted_by"]);
        let resp = acquire_handler(json!({ "ability_ura": taught_ura, "learner": "apprentice" }))
            .expect("learner acquires");

        // The learner owns a NEW ability under its own URA…
        let new_ura = resp["new_ura"].as_str().expect("new_ura");
        let sel = AbilitySelector::parse(new_ura).expect("new URA round-trips");
        assert_eq!(sel.owner_kind(), "agent");
        assert_eq!(sel.owner_ura(), apprentice_ura, "owner is the learner now");
        assert_eq!(resp["execution_mode"], EXECUTION_MODE_DEFAULT);

        // …the copy is on disk, and the original is untouched.
        let home = std::env::var("HOME").unwrap();
        assert!(std::path::Path::new(&home)
            .join("agents/apprentice/abilities/quote.ability.toml")
            .exists());
        assert!(std::path::Path::new(&home)
            .join("agents/mentor/abilities/quote.ability.toml")
            .exists());
    }

    #[test]
    fn learning_never_overwrites_an_existing_ability() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("teach");
        let home = std::env::var("HOME").unwrap();
        std::fs::write(
            std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml"),
            "name = \"quote\"\ndescription = \"the apprentice's own quote\"\n\n[input_schema]\ntype = \"object\"\n",
        )
        .expect("native manifest");
        let err = acquire_handler(json!({ "ability_ura": taught_ura, "learner": "apprentice" }))
            .expect_err("must refuse to clobber");
        assert!(format!("{err}").contains("never overwrites"), "{err}");
    }

    #[test]
    fn forget_removes_only_what_was_learned() {
        let _g = HomeGuard::new();
        let (taught_ura, apprentice_ura, mentor_ura) = seed();
        teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("teach");
        acquire_handler(json!({ "ability_ura": taught_ura, "learner": "apprentice" }))
            .expect("acquire");

        // A native ability is not forgettable…
        let err = forget_handler(json!({ "ability": "quote", "agent": "mentor" }))
            .expect_err("mentor never LEARNED quote");
        assert!(format!("{err}").contains("never learned"), "{err}");

        // …the learned copy is.
        forget_handler(json!({ "ability": "quote", "agent": "apprentice" })).expect("forget");
        let home = std::env::var("HOME").unwrap();
        assert!(!std::path::Path::new(&home)
            .join("agents/apprentice/abilities/quote.ability.toml")
            .exists());
        assert!(
            std::path::Path::new(&home)
                .join("agents/mentor/abilities/quote.ability.toml")
                .exists(),
            "the original survives a learner's forget"
        );
    }

    #[test]
    fn teach_refuses_unknown_abilities_precisely() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, mentor_ura) = seed();
        let err = teach_handler(
            caller_env(mentor_ura),
            json!({ "ability": "mentor.ghost", "learner_ura": apprentice_ura }),
        )
        .expect_err("no such manifest");
        assert!(format!("{err}").contains("publishes no ability"), "{err}");
    }

    #[test]
    fn teach_refuses_callers_that_are_not_the_owner_or_host_authority() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, _) = seed();
        let stranger = crate::ura::agent_ura("localhost", "dev", "stranger");

        let err = teach_handler(
            caller_env(stranger),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect_err("non-owner caller must not write a grant");
        assert!(
            format!("{err}").contains("cannot teach abilities owned by"),
            "{err}"
        );
    }

    #[test]
    fn teach_allows_the_host_device_authority_for_a_local_owner() {
        let _g = HomeGuard::new();
        let (_, apprentice_ura, _) = seed();
        let host = crate::ura::device_ura("localhost", "dev-1");

        let resp = teach_handler(
            caller_env(host.clone()),
            json!({ "ability": "mentor.quote", "learner_ura": apprentice_ura }),
        )
        .expect("host device signs for the local owner");
        assert_eq!(resp["granted_by"], host);
    }
}
