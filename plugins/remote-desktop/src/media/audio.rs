// EasyNet CLI — RemoteApp host-audio codec contract
// =================================================
//
// File: plugins/remote-desktop/src/media/audio.rs
// Description: Platform-neutral PCM-to-Opus packetization for RemoteApp.
//
// Protocol Responsibility:
// - None. Runtime Core and the RemoteDesktop session aggregate own authority,
//   transport epochs, receipts, cancellation, and terminal lifecycle.
//
// Implementation Approach:
// - Accept owned 48 kHz stereo float PCM chunks from a platform capture adapter.
// - Retain at most one partial 20 ms frame and emit bounded Opus packets.
//
// Usage Contract:
// - Platform adapters must normalize their native format before constructing a
//   `CapturedAudioChunk`; they must not substitute microphone input for host
//   system/application audio.
//
// Architectural Position:
// - RemoteDesktop plugin media codec layer, shared by macOS ScreenCaptureKit
//   and Windows/Linux host-audio adapters.

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use opus::{Application, Bitrate, Channels, Encoder};

pub(crate) const REMOTEAPP_AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub(crate) const REMOTEAPP_AUDIO_CHANNELS: usize = 2;
pub(crate) const REMOTEAPP_AUDIO_CODEC: &str = "opus";
pub(crate) const REMOTEAPP_AUDIO_PAYLOAD_CONTENT_TYPE: &str = "audio/opus";
// The in-process capture/encode pipeline below is superseded by the
// remoteapp media host for every production media profile; only the
// webrtc_audio pipeline tests still exercise it.
#[cfg(test)]
pub(crate) const REMOTEAPP_AUDIO_FRAME_DURATION_MS: u64 = 20;
#[cfg(test)]
pub(crate) const REMOTEAPP_AUDIO_SAMPLES_PER_CHANNEL: usize =
    REMOTEAPP_AUDIO_SAMPLE_RATE_HZ as usize * REMOTEAPP_AUDIO_FRAME_DURATION_MS as usize / 1_000;
#[cfg(test)]
pub(crate) const REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES: usize =
    REMOTEAPP_AUDIO_SAMPLES_PER_CHANNEL * REMOTEAPP_AUDIO_CHANNELS;
#[cfg(test)]
const MAX_OPUS_PACKET_BYTES: usize = 1_275;
#[cfg(test)]
const REMOTEAPP_AUDIO_BITRATE_BPS: i32 = 128_000;

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct CapturedAudioChunk {
    pub(crate) samples: Vec<f32>,
}

#[cfg(test)]
pub(crate) type AudioCaptureEvent = Result<CapturedAudioChunk, String>;
#[cfg(test)]
pub(crate) type AudioSink = Arc<dyn Fn(AudioCaptureEvent) + Send + Sync>;

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct EncodedOpusPacket {
    pub(crate) payload: Vec<u8>,
    pub(crate) duration: Duration,
}

#[cfg(test)]
pub(crate) struct RemoteAppOpusEncoder {
    encoder: Encoder,
    pending_samples: Vec<f32>,
}

#[cfg(test)]
impl RemoteAppOpusEncoder {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let mut encoder = Encoder::new(
            REMOTEAPP_AUDIO_SAMPLE_RATE_HZ,
            Channels::Stereo,
            Application::Audio,
        )?;
        encoder.set_bitrate(Bitrate::Bits(REMOTEAPP_AUDIO_BITRATE_BPS))?;
        encoder.set_vbr(true)?;
        Ok(Self {
            encoder,
            pending_samples: Vec::with_capacity(REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES),
        })
    }

    pub(crate) fn push_chunk(
        &mut self,
        chunk: CapturedAudioChunk,
    ) -> anyhow::Result<Vec<EncodedOpusPacket>> {
        self.pending_samples.extend(
            chunk
                .samples
                .into_iter()
                .map(|sample| sample.clamp(-1.0, 1.0)),
        );
        let frame_count = self.pending_samples.len() / REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES;
        let mut packets = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            let frame = &self.pending_samples[..REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES];
            let mut payload = vec![0_u8; MAX_OPUS_PACKET_BYTES];
            let encoded = self.encoder.encode_float(frame, &mut payload)?;
            payload.truncate(encoded);
            packets.push(EncodedOpusPacket {
                payload,
                duration: Duration::from_millis(REMOTEAPP_AUDIO_FRAME_DURATION_MS),
            });
            self.pending_samples
                .drain(..REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES);
        }
        Ok(packets)
    }

    #[cfg(test)]
    fn pending_sample_count(&self) -> usize {
        self.pending_samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(samples: usize) -> CapturedAudioChunk {
        CapturedAudioChunk {
            samples: vec![0.125; samples],
        }
    }

    #[test]
    fn opus_encoder_accumulates_exact_twenty_millisecond_frames() {
        let mut encoder = RemoteAppOpusEncoder::new().expect("Opus encoder");
        assert!(encoder
            .push_chunk(chunk(REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES / 2))
            .unwrap()
            .is_empty());
        let packets = encoder
            .push_chunk(chunk(REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES / 2))
            .unwrap();
        assert_eq!(packets.len(), 1);
        assert!(!packets[0].payload.is_empty());
        assert_eq!(
            packets[0].duration,
            Duration::from_millis(REMOTEAPP_AUDIO_FRAME_DURATION_MS)
        );
        assert_eq!(encoder.pending_sample_count(), 0);
    }

    #[test]
    fn opus_encoder_retains_only_one_partial_frame() {
        let mut encoder = RemoteAppOpusEncoder::new().expect("Opus encoder");
        let packets = encoder
            .push_chunk(chunk(REMOTEAPP_AUDIO_INTERLEAVED_SAMPLES * 3 + 17))
            .unwrap();
        assert_eq!(packets.len(), 3);
        assert_eq!(encoder.pending_sample_count(), 17);
    }
}
