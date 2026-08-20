use std::path::{Path, PathBuf};

use easynet_cli::daemon::ability::catalog::{
    ability_toml, voice_ability_contract_inventory, SystemAbilityContract,
};

type VoiceContractMutation = (&'static str, Box<dyn Fn(&mut SystemAbilityContract)>);

fn main() -> anyhow::Result<()> {
    let mut root = PathBuf::from(".");
    let mut self_test = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                root = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--root requires a path"))?,
                );
            }
            "--self-test" => self_test = true,
            _ => anyhow::bail!("usage: verify-voice-contract [--root <repository>] [--self-test]"),
        }
    }

    if self_test {
        run_self_tests()?;
    }
    verify_repository(&root)?;
    println!("verify-voice-contract: ok");
    Ok(())
}

fn verify_repository(root: &Path) -> anyhow::Result<()> {
    let expected = voice_contracts();
    let descriptor_root = root.join("ability-descriptors/system");
    for contract in &expected {
        let paths = descriptor_paths(&descriptor_root, &contract.name)?;
        let [path] = paths.as_slice() else {
            anyhow::bail!(
                "expected exactly one descriptor for {:?}, found {}: {:?}",
                contract.name,
                paths.len(),
                paths
            );
        };
        let body = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("read {}: {error}", path.display()))?;
        verify_contract(contract, &body)
            .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
    }

    Ok(())
}

fn voice_contracts() -> Vec<SystemAbilityContract> {
    voice_ability_contract_inventory(Default::default())
}

fn descriptor_paths(root: &Path, ability: &str) -> anyhow::Result<Vec<PathBuf>> {
    let expected_name = format!("{ability}.ability.toml");
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| anyhow::anyhow!("read {}: {error}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().and_then(|name| name.to_str())
                == Some(expected_name.as_str())
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    Ok(matches)
}

fn verify_contract(expected: &SystemAbilityContract, body: &str) -> anyhow::Result<()> {
    let actual = ability_toml::parse_ability_contract_toml(body)?;
    if &actual != expected {
        anyhow::bail!(
            "canonical descriptor mismatch for {:?}\nexpected: {expected:#?}\nactual: {actual:#?}",
            expected.name
        );
    }
    Ok(())
}

fn run_self_tests() -> anyhow::Result<()> {
    use easynet_cli::daemon::ability::conformance::CapabilityState;
    use easynet_cli::daemon::ability::descriptors::{
        AdmissionAction, ReceiptSemantics, ScopeRule, StateTransition, TransitionClass, Visibility,
    };
    use easynet_cli::daemon::ability::CallMode;

    let baseline = voice_contracts()
        .into_iter()
        .find(|contract| contract.name == "voice.report_metrics")
        .ok_or_else(|| anyhow::anyhow!("voice.report_metrics contract missing"))?;
    verify_contract(
        &baseline,
        &ability_toml::render_ability_contract_toml(&baseline),
    )?;

    let mut counterexamples: Vec<VoiceContractMutation> = vec![
        (
            "schema",
            Box::new(|c| c.input_schema = serde_json::json!({"type":"array"})),
        ),
        (
            "descriptor version",
            Box::new(|c| c.descriptor_version = "2.0.0".to_string()),
        ),
        (
            "description",
            Box::new(|c| c.description.push_str(" drift")),
        ),
        ("mode", Box::new(|c| c.call_mode = CallMode::Stream)),
        (
            "action",
            Box::new(|c| c.admission_action = AdmissionAction::Read),
        ),
        (
            "state",
            Box::new(|c| c.capability_state = CapabilityState::Unsupported),
        ),
        (
            "receipt",
            Box::new(|c| {
                c.receipt_semantics = ReceiptSemantics::StateTransition(
                    StateTransition::new("voice.report_metrics@v1", TransitionClass::Canonical)
                        .expect("valid self-test transition"),
                )
            }),
        ),
        (
            "visibility",
            Box::new(|c| c.visibility = Visibility::Public),
        ),
        (
            "subject scope",
            Box::new(|c| c.scope_subjects = ScopeRule::None),
        ),
        (
            "agent scope",
            Box::new(|c| c.scope_agents = ScopeRule::None),
        ),
        (
            "deny rules",
            Box::new(|c| c.denied_agents = vec!["easynet:///r/test/agent/denied".to_string()]),
        ),
        (
            "output receipt schema",
            Box::new(|c| c.output_receipt_schema = serde_json::json!({"type":"object"})),
        ),
        (
            "hints",
            Box::new(|c| c.hints.read_only = !c.hints.read_only),
        ),
    ];
    for (label, mutate) in counterexamples.drain(..) {
        let mut counterexample = baseline.clone();
        mutate(&mut counterexample);
        let body = ability_toml::render_ability_contract_toml(&counterexample);
        if verify_contract(&baseline, &body).is_ok() {
            anyhow::bail!("verifier accepted {label} counterexample");
        }
    }

    let without_action = ability_toml::render_ability_contract_toml(&baseline)
        .lines()
        .filter(|line| !line.starts_with("admission_action = "))
        .collect::<Vec<_>>()
        .join("\n");
    if ability_toml::parse_ability_contract_toml(&without_action).is_ok() {
        anyhow::bail!("verifier accepted a canonical descriptor without admission_action");
    }
    Ok(())
}
