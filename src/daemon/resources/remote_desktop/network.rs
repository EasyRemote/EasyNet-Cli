// EasyNet CLI — remote desktop network candidates
// ===============================================
//
// File: src/daemon/resources/remote_desktop/network.rs
// Description: Local interface candidate discovery for direct WebRTC endpoints.

#[cfg(unix)]
use std::net::Ipv4Addr;

pub(crate) fn direct_webrtc_udp_addrs() -> Vec<String> {
    direct_webrtc_host_ips()
        .into_iter()
        .map(|ip| format!("{ip}:0"))
        .collect()
}

fn direct_webrtc_host_ips() -> Vec<String> {
    let mut ips = vec!["127.0.0.1".to_string()];
    for candidate in local_interface_host_ips() {
        if !ips.iter().any(|addr| addr == &candidate) {
            ips.push(candidate);
        }
    }
    ips
}

#[cfg(unix)]
fn local_interface_host_ips() -> Vec<String> {
    let mut addrs = Vec::new();
    let mut ifaddr = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes a linked list owned by libc. Every
    // pointer is checked before dereference and the list is freed exactly once.
    unsafe {
        if libc::getifaddrs(&mut ifaddr) != 0 {
            return addrs;
        }

        let mut cursor = ifaddr;
        while !cursor.is_null() {
            let item = &*cursor;
            let sockaddr = item.ifa_addr;
            if !sockaddr.is_null() && (*sockaddr).sa_family as i32 == libc::AF_INET {
                let inet = *(sockaddr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(inet.sin_addr.s_addr));
                if !ip.is_unspecified() && !ip.is_loopback() {
                    addrs.push(ip.to_string());
                }
            }
            cursor = item.ifa_next;
        }
        libc::freeifaddrs(ifaddr);
    }

    addrs.sort();
    addrs.dedup();
    addrs
}

#[cfg(not(unix))]
fn local_interface_host_ips() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_webrtc_host_ips_are_local_interface_candidates_only() {
        let ips = direct_webrtc_host_ips();
        assert!(
            ips.iter().any(|ip| ip == "127.0.0.1"),
            "direct WebRTC candidates must always include loopback: {ips:?}"
        );
        assert!(
            !ips.iter().any(|ip| ip == "8.8.8.8"),
            "direct WebRTC candidates must not include or depend on public probe targets: {ips:?}"
        );
    }
}
