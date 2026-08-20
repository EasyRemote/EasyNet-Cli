pub const FS_READ: &str = "fs.read";
pub const FS_WRITE: &str = "fs.write";
pub const FS_STAT: &str = "fs.stat";
pub const FS_LIST: &str = "fs.list";
pub const FS_EDIT: &str = "fs.edit";
pub const FS_TRANSFER: &str = "fs.transfer";

pub const HTTP_REQUEST: &str = "http.request";
pub const PROCESS_EXEC: &str = "process.exec";
pub const SHELL_RUN: &str = "shell.run";

/// Device-sponsored SystemAgent id for baseline locomotion abilities:
/// filesystem, process, shell, HTTP egress, and file transfer.
pub const LOCOMOTION_SYSTEM_AGENT_ID: &str = "locomotion";

pub const SESSION_LIST: &str = crate::daemon::ability::runtime_admin_routes_gen::SESSION_LIST;
pub const SESSION_ATTACH: &str = "session.attach";
pub const SESSION_OPEN: &str = "session.open";

/// Device-sponsored SystemAgent id for daemon session observation/control
/// abilities. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.session`.
pub const SESSION_SYSTEM_AGENT_ID: &str = "session";

pub const NODE_DESCRIBE: &str = "node.describe";
pub const NODE_REMOVE: &str = "node.remove";

/// Device-sponsored SystemAgent id for node lifecycle and node directory
/// operations. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.node-management`.
pub const NODE_MANAGEMENT_SYSTEM_AGENT_ID: &str = "node-management";

pub const TERMINAL_ATTACH: &str = "terminal.attach";
pub const TERMINAL_CREATE: &str = "terminal.create";
pub const TERMINAL_LIST: &str = "terminal.list";
pub const TERMINAL_CLOSE: &str = "terminal.close";
pub const TERMINAL_INPUT: &str = "terminal.input";
pub const TERMINAL_READ: &str = "terminal.read";
pub const TERMINAL_RESIZE: &str = "terminal.resize";

/// Device-sponsored SystemAgent id for terminal/session PTY abilities. The
/// canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.terminal`.
pub const TERMINAL_SYSTEM_AGENT_ID: &str = "terminal";

pub const BASELINE_LOCOMOTION_PROFILE_VERSION: &str = "baseline-locomotion-v1";
