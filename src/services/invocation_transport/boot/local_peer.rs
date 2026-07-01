// EasyNet CLI — daemon Invocation local peer gate
// =================================================
//
// File: src/services/invocation_transport/boot/local_peer.rs
// Description: Verifies local Unix-domain-socket peers before the
//              daemon hands accepted streams to tonic.
//
// Protocol Responsibility:
// This file does not define Axon Invocation semantics. It protects the
// local transport boundary for `daemon.sock`: only same-uid local
// processes may reach the daemon Invocation gRPC service.
//
// Implementation Approach:
// Read OS-owned peer credentials from the accepted UDS stream
// (`SO_PEERCRED` on Linux, `getpeereid` on macOS/BSD-style targets)
// and compare the peer uid with the daemon process effective uid.
//
// Usage Contract:
// `LocalPeerGate::authorize_stream` must run before an accepted
// `UnixStream` is yielded to tonic. A rejected stream is dropped and
// must not enter `DaemonInvocationService`.
//
// Architectural Position:
// EasyNet-Cli daemon owns local device/runtime transport policy. The
// backend remains a product wrapper/client; Axon remains protocol
// truth and does not know about EasyNet's local socket policy.

use std::fmt;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};

use tokio::net::UnixStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PeerCredential {
    pub(super) uid: libc::uid_t,
    pub(super) gid: libc::gid_t,
    pub(super) pid: Option<libc::pid_t>,
}

#[derive(Debug)]
pub(super) enum PeerGateError {
    CredentialRead(io::Error),
    UidMismatch {
        expected_uid: libc::uid_t,
        credential: PeerCredential,
    },
}

impl PeerGateError {
    pub(super) fn credential(&self) -> Option<PeerCredential> {
        match self {
            Self::CredentialRead(_) => None,
            Self::UidMismatch { credential, .. } => Some(*credential),
        }
    }
}

impl fmt::Display for PeerGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CredentialRead(err) => write!(f, "failed to read UDS peer credentials: {err}"),
            Self::UidMismatch {
                expected_uid,
                credential,
            } => write!(
                f,
                "UDS peer uid {} rejected; daemon effective uid is {}",
                credential.uid, expected_uid
            ),
        }
    }
}

impl std::error::Error for PeerGateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CredentialRead(err) => Some(err),
            Self::UidMismatch { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LocalPeerGate {
    daemon_euid: libc::uid_t,
}

impl LocalPeerGate {
    pub(super) fn for_current_process() -> Self {
        Self {
            daemon_euid: unsafe { libc::geteuid() },
        }
    }

    pub(super) fn authorize_stream(
        &self,
        stream: &UnixStream,
    ) -> Result<PeerCredential, PeerGateError> {
        let credential =
            peer_credential_from_stream(stream).map_err(PeerGateError::CredentialRead)?;
        self.authorize_credential(credential)
    }

    fn authorize_credential(
        &self,
        credential: PeerCredential,
    ) -> Result<PeerCredential, PeerGateError> {
        if credential.uid == self.daemon_euid {
            Ok(credential)
        } else {
            Err(PeerGateError::UidMismatch {
                expected_uid: self.daemon_euid,
                credential,
            })
        }
    }
}

pub(super) fn peer_credential_from_stream(stream: &UnixStream) -> io::Result<PeerCredential> {
    peer_credential_from_fd(stream.as_raw_fd())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_credential_from_fd(fd: RawFd) -> io::Result<PeerCredential> {
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            cred.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if (len as usize) < std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SO_PEERCRED returned a short credential record",
        ));
    }
    let cred = unsafe { cred.assume_init() };
    Ok(PeerCredential {
        uid: cred.uid,
        gid: cred.gid,
        pid: Some(cred.pid),
    })
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn peer_credential_from_fd(fd: RawFd) -> io::Result<PeerCredential> {
    let mut uid = 0 as libc::uid_t;
    let mut gid = 0 as libc::gid_t;
    let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCredential {
        uid,
        gid,
        pid: None,
    })
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
fn peer_credential_from_fd(_fd: RawFd) -> io::Result<PeerCredential> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "UDS peer credential inspection is unsupported on this Unix target",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    impl LocalPeerGate {
        fn expecting_uid(uid: libc::uid_t) -> Self {
            Self { daemon_euid: uid }
        }
    }

    #[test]
    fn authorize_credential_accepts_matching_uid() {
        let gate = LocalPeerGate::expecting_uid(501);
        let credential = PeerCredential {
            uid: 501,
            gid: 20,
            pid: Some(1234),
        };

        assert_eq!(gate.authorize_credential(credential).unwrap(), credential);
    }

    #[test]
    fn authorize_credential_rejects_mismatched_uid() {
        let gate = LocalPeerGate::expecting_uid(501);
        let credential = PeerCredential {
            uid: 502,
            gid: 20,
            pid: Some(1234),
        };

        let err = gate.authorize_credential(credential).unwrap_err();
        match err {
            PeerGateError::UidMismatch {
                expected_uid,
                credential: actual,
            } => {
                assert_eq!(expected_uid, 501);
                assert_eq!(actual, credential);
            }
            other => panic!("expected uid mismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reads_same_process_peer_credentials_from_uds() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();

        let client_credential = peer_credential_from_stream(&client).unwrap();
        let server_credential = peer_credential_from_stream(&server).unwrap();
        let current_uid = unsafe { libc::geteuid() };

        assert_eq!(client_credential.uid, current_uid);
        assert_eq!(server_credential.uid, current_uid);
    }
}
