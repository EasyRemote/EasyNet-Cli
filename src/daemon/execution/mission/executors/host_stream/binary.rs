//! Versioned binary resident-host transport.
//!
//! Header layout (network byte order, 22 bytes):
//! `magic[4] | version:u8 | kind:u8 | flags:u16 | sequence:u64 |
//! content_type_len:u16 | payload_len:u32`.
//!
//! The request payload is JSON control data. Item payloads are opaque bytes
//! accompanied by an exact content type. Terminal payloads are the 32-byte
//! rolling digest. This keeps media out of JSON/base64 while retaining one
//! authenticated Axon Invocation and one deterministic stream terminal.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::contract::{HostStreamFailure, HostStreamFailureKind};

const MAGIC: &[u8; 4] = b"ERHS";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 22;
const KIND_REQUEST: u8 = 1;
const KIND_ITEM: u8 = 2;
const KIND_TERMINAL: u8 = 3;
const KIND_ERROR: u8 = 4;
const MAX_CONTENT_TYPE_LEN: usize = 1024;
pub(crate) const MAX_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum BinaryHostFrame {
    Item {
        sequence: u64,
        content_type: String,
        payload: Vec<u8>,
    },
    Terminal {
        frames: u64,
        output_hash: [u8; 32],
    },
    Error(Value),
}

#[derive(Debug, Clone)]
pub(crate) struct BinaryHashState {
    previous: [u8; 32],
    frames: u64,
}

impl BinaryHashState {
    pub(crate) fn new() -> Self {
        Self {
            previous: Sha256::digest(b"").into(),
            frames: 0,
        }
    }

    pub(crate) fn fold(
        &mut self,
        sequence: u64,
        content_type: &str,
        payload: &[u8],
    ) -> Result<(), HostStreamFailure> {
        if sequence != self.frames {
            return Err(failure(
                HostStreamFailureKind::StreamTruncated,
                format!(
                    "binary frame reorder/gap: expected seq {}, got {sequence}",
                    self.frames
                ),
            ));
        }
        let content_type_bytes = content_type.as_bytes();
        let content_type_len = u16::try_from(content_type_bytes.len()).map_err(|_| {
            failure(
                HostStreamFailureKind::Protocol,
                "binary frame content type exceeds u16 length".to_string(),
            )
        })?;
        let mut hasher = Sha256::new();
        hasher.update(self.previous);
        hasher.update(sequence.to_be_bytes());
        hasher.update(content_type_len.to_be_bytes());
        hasher.update(content_type_bytes);
        hasher.update(payload);
        self.previous = hasher.finalize().into();
        self.frames += 1;
        Ok(())
    }

    pub(crate) fn verify_terminal(
        &self,
        frames: u64,
        output_hash: &[u8; 32],
    ) -> Result<(), HostStreamFailure> {
        if frames != self.frames {
            return Err(failure(
                HostStreamFailureKind::StreamTruncated,
                format!(
                    "binary terminal frame count {frames} != frames received {}",
                    self.frames
                ),
            ));
        }
        if output_hash != &self.previous {
            return Err(failure(
                HostStreamFailureKind::StreamTruncated,
                format!(
                    "binary output hash mismatch: host sha256:{} != computed sha256:{}",
                    hex::encode(output_hash),
                    hex::encode(self.previous),
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) async fn write_request<W: AsyncWrite + Unpin>(
    writer: &mut W,
    request: &Value,
) -> Result<(), HostStreamFailure> {
    let payload = serde_json::to_vec(request).map_err(|error| {
        failure(
            HostStreamFailureKind::Internal,
            format!("encode binary host request: {error}"),
        )
    })?;
    write_frame(writer, KIND_REQUEST, 0, "application/json", &payload).await
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<BinaryHostFrame, HostStreamFailure> {
    let mut header = [0_u8; HEADER_LEN];
    reader.read_exact(&mut header).await.map_err(|error| {
        failure(
            HostStreamFailureKind::StreamTruncated,
            format!("read binary host frame header: {error}"),
        )
    })?;
    if &header[0..4] != MAGIC {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            "binary host frame has invalid magic".to_string(),
        ));
    }
    if header[4] != VERSION {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            format!("unsupported binary host protocol version {}", header[4]),
        ));
    }
    let kind = header[5];
    let flags = u16::from_be_bytes([header[6], header[7]]);
    if flags != 0 {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            format!("binary host frame has unsupported flags {flags:#06x}"),
        ));
    }
    let sequence = u64::from_be_bytes(
        header[8..16]
            .try_into()
            .expect("binary header sequence slice has fixed width"),
    );
    let content_type_len = u16::from_be_bytes([header[16], header[17]]) as usize;
    let payload_len = u32::from_be_bytes(
        header[18..22]
            .try_into()
            .expect("binary header payload slice has fixed width"),
    ) as usize;
    if content_type_len > MAX_CONTENT_TYPE_LEN {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            format!("binary host content type length {content_type_len} exceeds limit"),
        ));
    }
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            format!("binary host payload length {payload_len} exceeds limit"),
        ));
    }
    let mut content_type_bytes = vec![0_u8; content_type_len];
    reader
        .read_exact(&mut content_type_bytes)
        .await
        .map_err(|error| {
            failure(
                HostStreamFailureKind::StreamTruncated,
                format!("read binary host content type: {error}"),
            )
        })?;
    let content_type = String::from_utf8(content_type_bytes).map_err(|error| {
        failure(
            HostStreamFailureKind::Protocol,
            format!("binary host content type is not UTF-8: {error}"),
        )
    })?;
    let mut payload = vec![0_u8; payload_len];
    reader.read_exact(&mut payload).await.map_err(|error| {
        failure(
            HostStreamFailureKind::StreamTruncated,
            format!("read binary host payload: {error}"),
        )
    })?;

    match kind {
        KIND_ITEM => {
            if content_type.trim().is_empty() {
                return Err(failure(
                    HostStreamFailureKind::Protocol,
                    "binary item content type is empty".to_string(),
                ));
            }
            Ok(BinaryHostFrame::Item {
                sequence,
                content_type,
                payload,
            })
        }
        KIND_TERMINAL => {
            if !content_type.is_empty() || payload.len() != 32 {
                return Err(failure(
                    HostStreamFailureKind::Protocol,
                    "binary terminal requires empty content type and 32-byte hash".to_string(),
                ));
            }
            Ok(BinaryHostFrame::Terminal {
                frames: sequence,
                output_hash: payload
                    .try_into()
                    .expect("binary terminal payload length was checked"),
            })
        }
        KIND_ERROR => {
            if content_type != "application/json" {
                return Err(failure(
                    HostStreamFailureKind::Protocol,
                    "binary error content type must be application/json".to_string(),
                ));
            }
            let error = serde_json::from_slice(&payload).map_err(|error| {
                failure(
                    HostStreamFailureKind::Protocol,
                    format!("binary host error payload is not JSON: {error}"),
                )
            })?;
            Ok(BinaryHostFrame::Error(error))
        }
        KIND_REQUEST => Err(failure(
            HostStreamFailureKind::Protocol,
            "host sent a request frame to the daemon".to_string(),
        )),
        other => Err(failure(
            HostStreamFailureKind::Protocol,
            format!("binary host frame has unsupported kind {other}"),
        )),
    }
}

async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    kind: u8,
    sequence: u64,
    content_type: &str,
    payload: &[u8],
) -> Result<(), HostStreamFailure> {
    if content_type.len() > MAX_CONTENT_TYPE_LEN || payload.len() > MAX_PAYLOAD_LEN {
        return Err(failure(
            HostStreamFailureKind::Protocol,
            "binary host request exceeds frame bounds".to_string(),
        ));
    }
    let content_type_len = u16::try_from(content_type.len()).map_err(|_| {
        failure(
            HostStreamFailureKind::Protocol,
            "binary host content type exceeds u16 length".to_string(),
        )
    })?;
    let payload_len = u32::try_from(payload.len()).map_err(|_| {
        failure(
            HostStreamFailureKind::Protocol,
            "binary host payload exceeds u32 length".to_string(),
        )
    })?;
    let mut header = [0_u8; HEADER_LEN];
    header[0..4].copy_from_slice(MAGIC);
    header[4] = VERSION;
    header[5] = kind;
    header[8..16].copy_from_slice(&sequence.to_be_bytes());
    header[16..18].copy_from_slice(&content_type_len.to_be_bytes());
    header[18..22].copy_from_slice(&payload_len.to_be_bytes());
    writer.write_all(&header).await.map_err(write_failure)?;
    writer
        .write_all(content_type.as_bytes())
        .await
        .map_err(write_failure)?;
    writer.write_all(payload).await.map_err(write_failure)?;
    writer.flush().await.map_err(write_failure)
}

fn write_failure(error: std::io::Error) -> HostStreamFailure {
    failure(
        HostStreamFailureKind::HostUnreachable,
        format!("write binary host frame: {error}"),
    )
}

fn failure(kind: HostStreamFailureKind, message: String) -> HostStreamFailure {
    HostStreamFailure::new(kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_frame_round_trips_through_exact_header() {
        let mut bytes = Vec::new();
        write_request(
            &mut bytes,
            &serde_json::json!({"request":{"fn":"er.frames","args":{}}}),
        )
        .await
        .unwrap();
        assert_eq!(&bytes[0..4], b"ERHS");
        assert_eq!(bytes[4], 1);
        assert_eq!(bytes[5], KIND_REQUEST);
    }

    #[tokio::test]
    async fn decoder_rejects_payload_length_before_allocating() {
        let mut bytes = vec![0_u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(MAGIC);
        bytes[4] = VERSION;
        bytes[5] = KIND_ITEM;
        bytes[18..22].copy_from_slice(&((MAX_PAYLOAD_LEN as u32) + 1).to_be_bytes());
        let error = read_frame(&mut bytes.as_slice()).await.unwrap_err();
        assert_eq!(error.kind, HostStreamFailureKind::Protocol);
        assert!(error.message.contains("exceeds limit"));
    }

    #[test]
    fn typed_hash_detects_content_type_substitution() {
        let mut image = BinaryHashState::new();
        image.fold(0, "image/jpeg", b"same bytes").unwrap();
        let mut audio = BinaryHashState::new();
        audio.fold(0, "audio/mpeg", b"same bytes").unwrap();
        assert_ne!(image.previous, audio.previous);
    }
}
