//! Linux X11 window-owner resolution.
//!
//! `_NET_WM_PID` is advisory and optional. X-Resource 1.2 can instead ask the
//! X server for the kernel-derived PID of the local client that owns a window
//! XID. RemoteApp uses this only to project process-scoped Application
//! Resources; the XID remains the exact Window capture locator.

use xcb::res;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinuxProcessInstance {
    pid: u32,
    start_ticks: u64,
    boot_id: String,
}

impl LinuxProcessInstance {
    pub(crate) fn resolve(pid: u32) -> anyhow::Result<Self> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|error| anyhow::anyhow!("read /proc/{pid}/stat: {error}"))?;
        let start_ticks = parse_linux_process_start_ticks(&stat)?;
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|error| anyhow::anyhow!("read Linux boot id: {error}"))?
            .trim()
            .to_string();
        anyhow::ensure!(!boot_id.is_empty(), "Linux boot id is empty");
        Ok(Self {
            pid,
            start_ticks,
            boot_id,
        })
    }

    pub(crate) const fn start_ticks(&self) -> u64 {
        self.start_ticks
    }

    pub(crate) fn boot_id(&self) -> &str {
        &self.boot_id
    }

    pub(crate) fn stable_id(&self) -> String {
        format!("linux:{}:{}:{}", self.boot_id, self.pid, self.start_ticks)
    }
}

fn parse_linux_process_start_ticks(stat: &str) -> anyhow::Result<u64> {
    // `/proc/<pid>/stat` field 2 is a parenthesized command and may itself
    // contain spaces or `)`. Split after the final `)`; field 3 (`state`) is
    // then index 0 and field 22 (`starttime`) is index 19.
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing command terminator"))?;
    let fields = stat[command_end + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();
    let start_ticks = fields
        .get(19)
        .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing starttime field"))?
        .parse::<u64>()
        .map_err(|error| anyhow::anyhow!("parse Linux process starttime: {error}"))?;
    anyhow::ensure!(start_ticks > 0, "Linux process starttime must be positive");
    Ok(start_ticks)
}

pub(crate) struct LinuxX11WindowOwnerResolver {
    connection: xcb::Connection,
}

impl LinuxX11WindowOwnerResolver {
    pub(crate) fn connect() -> anyhow::Result<Self> {
        let (connection, _) =
            xcb::Connection::connect_with_extensions(None, &[xcb::Extension::Res], &[])
                .map_err(|error| anyhow::anyhow!("connect to X server with X-Resource: {error}"))?;
        let version = connection
            .wait_for_reply(connection.send_request(&res::QueryVersion {
                client_major: 1,
                client_minor: 2,
            }))
            .map_err(|error| anyhow::anyhow!("query X-Resource version: {error}"))?;
        if (version.server_major(), version.server_minor()) < (1, 2) {
            anyhow::bail!(
                "X-Resource 1.2 is required for local client PID resolution, server provides {}.{}",
                version.server_major(),
                version.server_minor()
            );
        }
        Ok(Self { connection })
    }

    pub(crate) fn resolve_local_client_pid(&self, window_id: u32) -> anyhow::Result<Option<u32>> {
        let specs = [res::ClientIdSpec {
            client: window_id,
            mask: res::ClientIdMask::LOCAL_CLIENT_PID,
        }];
        let reply = self
            .connection
            .wait_for_reply(
                self.connection
                    .send_request(&res::QueryClientIds { specs: &specs }),
            )
            .map_err(|error| {
                anyhow::anyhow!("query X-Resource local client PID for window {window_id}: {error}")
            })?;
        Ok(reply.ids().find_map(|client_id| {
            let spec = client_id.spec();
            // X-Resource normalizes `spec.client` to the owning client's XID
            // base, so it need not equal the queried window XID. The request
            // contains exactly one XID; select its LocalClientPID value by mask.
            spec.mask
                .contains(res::ClientIdMask::LOCAL_CLIENT_PID)
                .then(|| client_id.value().first().copied())
                .flatten()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_starttime_parser_handles_spaces_and_parentheses_in_comm() {
        let stat =
            "4242 (EasyNet Remote) App) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 20";
        assert_eq!(parse_linux_process_start_ticks(stat).unwrap(), 987654);
    }

    #[test]
    fn process_starttime_parser_rejects_truncated_or_zero_identity() {
        assert!(parse_linux_process_start_ticks("42 broken").is_err());
        assert!(parse_linux_process_start_ticks(
            "42 (app) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 0"
        )
        .is_err());
    }
}
