// EasyNet CLI — Network & Process Utilities
// ==========================================
//
// File: src/shared/net.rs
// Description: Shared process lifecycle functions used by start.rs and stop.rs.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Check whether a process with the given PID is still alive.
/// Returns true if the process exists (even if we lack permission to signal it).
#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    let ret = unsafe { libc::kill(pid, 0) };
    if ret == 0 {
        return true;
    }
    // ESRCH = no such process. Any other errno (e.g. EPERM) means the process exists.
    let err = std::io::Error::last_os_error();
    err.raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
pub fn is_pid_alive(_pid: u32) -> bool {
    // Cannot check on non-unix; assume alive to be safe.
    true
}

/// Check whether a process with the given PID is an EasyNet binary.
/// Used to reduce PID-reuse risk before signaling.
/// Returns `true` if the process command line contains "easynet", or if we cannot determine it.
pub fn is_easynet_process(pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output();
        if let Ok(out) = out {
            let comm = String::from_utf8_lossy(&out.stdout);
            return comm.contains("easynet");
        }
    }
    #[cfg(target_os = "linux")]
    {
        let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline"));
        if let Ok(cmdline) = cmdline {
            return cmdline.contains("easynet");
        }
    }
    // Cannot determine — assume it's ours to avoid breaking stop on unknown platforms.
    true
}

/// Send SIGTERM to a process and wait up to `timeout` for it to exit.
/// Escalates to SIGKILL if the process does not exit in time.
/// Returns `true` if the process exited.
pub fn kill_and_wait(pid: u32, timeout: std::time::Duration) -> bool {
    #[cfg(unix)]
    {
        let Ok(raw_pid) = i32::try_from(pid) else {
            return false;
        };
        unsafe { libc::kill(raw_pid, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !is_pid_alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if is_pid_alive(pid) {
            eprintln!("  pid {pid} did not exit after SIGTERM, sending SIGKILL");
            unsafe { libc::kill(raw_pid, libc::SIGKILL) };
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        !is_pid_alive(pid)
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(500));
        !is_pid_alive(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, timeout);
        false
    }
}
