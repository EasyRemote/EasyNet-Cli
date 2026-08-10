pub const CHAT: &str = "chat";
pub const DISCOVER_VERB: &str = "discover";
pub const DISCOVER: &str = "agent.discover";
pub const INVOKE: &str = "invoke";

pub const AGENT_LIST: &str = "agent.list";
pub const AGENT_START: &str = "agent.start";
pub const AGENT_STOP: &str = "agent.stop";
pub const AGENT_PURGE: &str = "agent.purge";
pub const AGENT_PURGE_RECONCILE: &str = "agent.purge.reconcile";
pub const AGENT_REFRESH: &str = "agent.refresh";
pub const AGENT_ABILITY_PUT: &str = "agent.ability.put";

pub const CHAT_HISTORY_LIST: &str = "chat.history.list";
pub const CHAT_HISTORY_GET: &str = "chat.history.get";

/// Device-sponsored SystemAgent id for daemon-native Agent management
/// abilities. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.agent-management`.
pub const AGENT_MANAGEMENT_SYSTEM_AGENT_ID: &str = "agent-management";
