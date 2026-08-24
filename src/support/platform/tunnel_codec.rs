//! EasyNet tunnel binary framing.
//! ===============================
//!
//! File: src/support/platform/tunnel_codec.rs
//! Description: Shared zero-encoding-overhead framing for multiplexed tunnel bytes.
//!
//! Protocol Responsibility:
//! - Keep tunnel data as native bytes across CLI, daemon, and Axon Bidi transport.
//! - Bind listener-mode bytes to one UUID connection without JSON or Base64.
//!
//! Implementation Approach:
//! - Connect mode carries payload bytes unchanged.
//! - Listen mode prefixes the 16 raw UUID bytes to each payload chunk.
//!
//! Usage Contract:
//! - Callers enforce their configured payload-size cap after decoding.
//! - Connection identifiers must be canonical UUID strings.
//!
//! Architectural Position:
//! - Product transport codec shared by the daemon ability and CLI adapter.

pub(crate) const TUNNEL_DATA_CONTENT_TYPE: &str = "application/vnd.easynet.tunnel-data";
const CONNECTION_HEADER_BYTES: usize = 16;

pub(crate) fn encode_multiplexed_data(
    connection_id: &str,
    payload: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let id = uuid::Uuid::parse_str(connection_id)
        .map_err(|error| anyhow::anyhow!("TUNNEL_STATE_ERROR: invalid connection id: {error}"))?;
    let mut frame = Vec::with_capacity(CONNECTION_HEADER_BYTES + payload.len());
    frame.extend_from_slice(id.as_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub(crate) fn decode_multiplexed_data(mut frame: Vec<u8>) -> anyhow::Result<(String, Vec<u8>)> {
    anyhow::ensure!(
        frame.len() >= CONNECTION_HEADER_BYTES,
        "FRAME_SEQUENCE_ERROR: tunnel data frame omitted connection header"
    );
    let connection_id = uuid::Uuid::from_slice(&frame[..CONNECTION_HEADER_BYTES])
        .map_err(|error| {
            anyhow::anyhow!("FRAME_SEQUENCE_ERROR: invalid connection header: {error}")
        })?
        .to_string();
    let payload = frame.split_off(CONNECTION_HEADER_BYTES);
    Ok((connection_id, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplexed_data_round_trips_all_byte_values() {
        let id = "9d0ac55a-6aa8-4b57-8d68-d5e880f040b0";
        let payload = (0_u8..=u8::MAX).collect::<Vec<_>>();
        let frame = encode_multiplexed_data(id, &payload).expect("encode");
        let (decoded_id, decoded) = decode_multiplexed_data(frame).expect("decode");
        assert_eq!(decoded_id, id);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn multiplexed_data_rejects_truncated_header() {
        assert!(decode_multiplexed_data(vec![0; 15]).is_err());
    }
}
