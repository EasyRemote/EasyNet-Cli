//! Agent and Mission/EAL execution support owned by the daemon runtime.

pub(crate) mod adapter;
pub(crate) mod agent_ability_specs;
pub(crate) mod context;
pub(crate) mod directory;
pub mod discuss;
pub(crate) mod dispatch;
pub(crate) mod drivers;
pub(crate) mod executors;
pub(crate) mod invocation_gateway;
pub mod orchestration;
pub(crate) mod persisted_identity;
pub(crate) mod process_runner;
pub(crate) mod run_store;
pub(crate) mod session;
pub(crate) mod stream_ui;
pub(crate) mod timeline;
pub(crate) mod toml_escape;
pub(crate) mod workspace;

pub mod failure_codes;

#[must_use = "mission context only stays installed while the returned guard is alive"]
pub fn enter_mission_context_for_current_thread(
    mission_id: impl Into<String>,
    mission_run_dir: impl Into<std::path::PathBuf>,
) -> impl Drop {
    context::enter(context::DispatchContext::for_mission(
        mission_id,
        mission_run_dir.into(),
    ))
}
