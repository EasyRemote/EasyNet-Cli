//! LaunchServices bootstrap for the macOS media-host application.
//!
//! LaunchServices does not inherit the daemon's private file descriptors. The
//! daemon therefore authenticates a same-user Unix-domain connection and passes
//! the already-created bounded lanes with `SCM_RIGHTS` before protocol bytes
//! are exchanged. No Runtime authority or user data enters this bootstrap.

use std::io;
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

pub const ARG: &str = "--easynet-launch-services-bootstrap";
pub const FILE_DESCRIPTOR_COUNT: usize = 8;
const MAGIC: &[u8] = b"easynet-remoteapp-ls-v1";

pub fn send_file_descriptors(stream: &UnixStream, descriptors: &[RawFd]) -> io::Result<()> {
    if descriptors.len() != FILE_DESCRIPTOR_COUNT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid LaunchServices descriptor count",
        ));
    }
    ensure_same_user_peer(stream)?;
    let mut payload = MAGIC.to_vec();
    let mut iovec = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    let control_len = unsafe {
        libc::CMSG_SPACE((descriptors.len() * size_of::<RawFd>()) as libc::c_uint) as usize
    };
    let mut control = vec![0_u8; control_len];
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() as _;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null() {
            return Err(io::Error::other("missing LaunchServices control header"));
        }
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len =
            libc::CMSG_LEN((descriptors.len() * size_of::<RawFd>()) as libc::c_uint);
        std::ptr::copy_nonoverlapping(
            descriptors.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(header),
            descriptors.len() * size_of::<RawFd>(),
        );
        let sent = libc::sendmsg(stream.as_raw_fd(), &message, 0);
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        if sent as usize != payload.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short LaunchServices bootstrap write",
            ));
        }
    }
    Ok(())
}

pub fn receive_file_descriptors(stream: &UnixStream) -> io::Result<Vec<OwnedFd>> {
    ensure_same_user_peer(stream)?;
    let mut payload = vec![0_u8; MAGIC.len()];
    let mut iovec = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    let control_len = unsafe {
        libc::CMSG_SPACE((FILE_DESCRIPTOR_COUNT * size_of::<RawFd>()) as libc::c_uint) as usize
    };
    let mut control = vec![0_u8; control_len];
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() as _;
    let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated LaunchServices ancillary data",
        ));
    }
    if received as usize != MAGIC.len() || payload != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid LaunchServices bootstrap magic",
        ));
    }
    let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    if header.is_null()
        || unsafe { (*header).cmsg_level } != libc::SOL_SOCKET
        || unsafe { (*header).cmsg_type } != libc::SCM_RIGHTS
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LaunchServices bootstrap omitted file descriptors",
        ));
    }
    let data_bytes = unsafe { (*header).cmsg_len }
        .checked_sub(unsafe { libc::CMSG_LEN(0) })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid ancillary length"))?
        as usize;
    if data_bytes != FILE_DESCRIPTOR_COUNT * size_of::<RawFd>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid LaunchServices ancillary descriptor count",
        ));
    }
    let raw = unsafe {
        std::slice::from_raw_parts(
            libc::CMSG_DATA(header).cast::<RawFd>(),
            FILE_DESCRIPTOR_COUNT,
        )
    };
    Ok(raw
        .iter()
        .copied()
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect())
}

fn ensure_same_user_peer(stream: &UnixStream) -> io::Result<()> {
    let mut peer_user: libc::uid_t = 0;
    let mut peer_group: libc::gid_t = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer_user, &mut peer_group) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if peer_user != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "LaunchServices bootstrap peer belongs to a different user",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    #[test]
    fn transfers_the_exact_bounded_descriptor_set() {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let files = (0..FILE_DESCRIPTOR_COUNT)
            .map(|_| File::open("/dev/null").unwrap())
            .collect::<Vec<_>>();
        let raw = files.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>();

        send_file_descriptors(&sender, &raw).unwrap();
        let received = receive_file_descriptors(&receiver).unwrap();

        assert_eq!(received.len(), FILE_DESCRIPTOR_COUNT);
        assert!(received
            .iter()
            .all(|descriptor| unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) } >= 0));
    }

    #[test]
    fn rejects_an_unbounded_descriptor_set() {
        let (sender, _receiver) = UnixStream::pair().unwrap();
        let error = send_file_descriptors(&sender, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
