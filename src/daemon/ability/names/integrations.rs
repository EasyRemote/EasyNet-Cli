pub const A2A_BRIDGE_LIST_SKILLS: &str = "a2a.bridge.list_skills";
pub const A2A_BRIDGE_SEND_TASK: &str = "a2a.bridge.send_task";
pub const A2A_CLIENT_SEND_TASK: &str = "a2a.client.send_task";

/// Device-sponsored SystemAgent id for the daemon-local A2A integration
/// adapter. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.a2a-integration`.
pub const A2A_INTEGRATION_SYSTEM_AGENT_ID: &str = "a2a-integration";

pub const MCP_BRIDGE_LIST_TOOLS: &str = "mcp.bridge.list_tools";
pub const MCP_BRIDGE_CALL_TOOL: &str = "mcp.bridge.call_tool";
pub const MCP_CLIENT_LIST: &str = "mcp.client.list";
pub const MCP_CLIENT_CALL: &str = "mcp.client.call";

/// Device-sponsored SystemAgent id for inbound/outbound MCP edge adaptation
/// and reflected tools backed by MCP processes configured on this Device.
pub const MCP_INTEGRATION_SYSTEM_AGENT_ID: &str = "mcp-integration";

pub const OPENAI_CHAT_COMPLETIONS: &str = "openai.chat_completions";
pub const OPENAI_LIST_MODELS: &str = "openai.list_models";
pub const OPENAI_FILES_UPLOAD: &str = "openai.files.upload";
pub const OPENAI_FILES_RETRIEVE: &str = "openai.files.retrieve";
pub const OPENAI_FILES_DELETE: &str = "openai.files.delete";

/// Device-sponsored SystemAgent id for the daemon-local OpenAI compatibility
/// adapter. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.openai-compat`.
pub const OPENAI_COMPAT_SYSTEM_AGENT_ID: &str = "openai-compat";

pub const PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID: &str = "plugin-management";
pub const PLUGIN_RELOAD: &str = "plugin.reload";
pub const PLUGIN_STATUS: &str = "plugin.status";
pub const PLUGIN_ACTIVATE_REALTIME: &str = "plugin.activate_realtime";
pub const PLUGIN_COMPANION_STATUS: &str = "plugin.companion_status";
pub const PLUGIN_COMPANION_RECONCILE: &str = "plugin.companion_reconcile";

/// Device-sponsored SystemAgent id for the remote desktop product surface.
/// The remote desktop plugin contributes AbilityImpls; public
/// `remote_desktop.*` descriptors are owned by this SystemAgent, not by
/// plugin-management.
pub const REMOTE_DESKTOP_SYSTEM_AGENT_ID: &str = "remote-desktop";
