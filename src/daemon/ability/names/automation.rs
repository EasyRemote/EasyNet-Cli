/// Device-sponsored SystemAgent id for daemon-native automation and
/// orchestration abilities. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.automation`.
/// Mission/EAL remains an implementation strategy for composite abilities; this
/// SystemAgent owns the public control surface.
pub const AUTOMATION_SYSTEM_AGENT_ID: &str = "automation";

pub const DISCUSS_CREATE: &str = "discuss.create";
pub const DISCUSS_POST: &str = "discuss.post";
pub const DISCUSS_SUBSCRIBE: &str = "discuss.subscribe";
pub const DISCUSS_LIST_TURNS: &str = "discuss.list_turns";

pub const LOOP_CREATE: &str = "loop.create";
pub const LOOP_STATUS: &str = "loop.status";
pub const LOOP_SUBSCRIBE: &str = "loop.subscribe";
pub const LOOP_CANCEL: &str = "loop.cancel";

pub const MISSION_RUN: &str = "mission.run";
pub const MISSION_TRACK: &str = "mission.track";
pub const MISSION_CANCEL: &str = "mission.cancel";
pub const MISSION_EVENTS: &str = "mission.events";
pub const MISSION_DISCUSS_ROUND: &str = "mission.discuss_round";
pub const MISSION_THINK: &str = "mission.think";

pub const SCHEDULE_ADD: &str = "schedule.add";
pub const SCHEDULE_LIST: &str = "schedule.list";
pub const SCHEDULE_REMOVE: &str = "schedule.remove";
pub const SCHEDULE_ENABLE: &str = "schedule.enable";
