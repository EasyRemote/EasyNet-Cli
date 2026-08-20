use std::fs;
use std::path::PathBuf;

fn repo_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join(path)).expect("read RF-2 boundary source")
}

#[test]
fn production_mission_and_eal_have_one_invocation_authority() {
    let orchestration = repo_file("src/daemon/execution/mission/orchestration.rs");
    let gateway = repo_file("src/daemon/execution/mission/invocation_gateway.rs");
    let eal_dispatch = repo_file("src/eal/interpreter/dispatch.rs");
    let eal_interpreter = repo_file("src/eal/interpreter/mod.rs");
    let eal_executor = repo_file("src/daemon/execution/mission/executors/eal.rs");
    let mission_ability = repo_file("src/daemon/ability/builtins/automation/mission.rs");
    let hosted_agent = repo_file("src/daemon/ability/builtins/agents/chat.rs");
    let plugin_host = repo_file("src/daemon/plugins/host_api.rs");

    assert!(!orchestration.contains("fn run_mission_inproc"));
    assert!(!eal_dispatch.contains("invoke_local_ability"));
    assert!(!eal_dispatch.contains("invoke_remote_target"));
    assert!(!eal_interpreter.contains("pub fn execute_with_dispatcher_for_trace_with_timeout"));
    assert!(!eal_executor.contains("ParentInvocationContext"));
    assert!(!eal_executor.contains("__axon_invocation"));
    assert!(eal_dispatch.contains(".invoke(request)"));
    assert!(!gateway.contains("invoke_step"));
    assert!(!gateway.contains("anyhow::Result<Value>;"));
    assert!(mission_ability.contains("register_rpc_with_envelope_and_owner"));
    assert!(mission_ability.contains("from_admitted_envelope"));
    assert!(hosted_agent.contains("from_admitted_envelope"));
    assert!(plugin_host.contains("from_admitted_envelope"));
}

#[test]
fn cli_execution_surfaces_only_call_mission_run() {
    for (path, mission_gateway) in [
        (
            "src/cli/commands/groups/mission.rs",
            "automation::mission::ABILITY_RUN",
        ),
        (
            "src/cli/commands/agent/send.rs",
            "invoke_agent_subject_mission_run",
        ),
    ] {
        let source = repo_file(path);
        assert!(source.contains(mission_gateway), "{path}");
        assert!(!source.contains("MissionRunner"), "{path}");
        assert!(!source.contains("run_mission_inproc"), "{path}");
        assert!(!source.contains("send_external"), "{path}");
    }
}
