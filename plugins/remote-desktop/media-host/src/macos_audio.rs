// EasyNet CLI — macOS hosted system/application audio
// ====================================================
//
// Extracts the exact ScreenCaptureKit stream's 48 kHz stereo PCM and emits
// bounded 20 ms Opus packets. The enclosing media-host session owns capture
// filtering, media gates and process lifecycle; the daemon owns WebRTC,
// authority, receipts and session state.

#![cfg(all(feature = "native-media", target_os = "macos"))]

use std::mem::{align_of, size_of};
use std::ptr::NonNull;
use std::slice;
use std::sync::Arc;

use objc2_core_audio_types::{
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
    kAudioFormatLinearPCM, AudioBuffer, AudioBufferList, AudioStreamBasicDescription,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer,
};
use opus::{Application, Bitrate, Channels, Encoder};

pub(super) const SAMPLE_RATE_HZ: u32 = 48_000;
pub(super) const CHANNELS: usize = 2;
pub(super) const FRAME_DURATION_MS: u64 = 20;
pub(super) const SAMPLES_PER_CHANNEL: usize =
    SAMPLE_RATE_HZ as usize * FRAME_DURATION_MS as usize / 1_000;
const INTERLEAVED_SAMPLES_PER_FRAME: usize = SAMPLES_PER_CHANNEL * CHANNELS;
const MAX_OPUS_PACKET_BYTES: usize = 1_275;
const BITRATE_BPS: i32 = 128_000;
const AUDIO_BUFFER_LIST_ALIGNMENT: usize = align_of::<AudioBufferList>();

#[derive(Debug)]
pub(super) struct CapturedAudioChunk {
    pub(super) samples: Vec<f32>,
}

pub(super) type AudioCaptureEvent = Result<CapturedAudioChunk, String>;
pub(super) type AudioSink = Arc<dyn Fn(AudioCaptureEvent) + Send + Sync>;

#[derive(Debug)]
pub(super) struct EncodedOpusPacket {
    pub(super) payload: Vec<u8>,
}

pub(super) struct OpusPacketizer {
    encoder: Encoder,
    pending_samples: Vec<f32>,
}

impl OpusPacketizer {
    pub(super) fn new() -> anyhow::Result<Self> {
        let mut encoder = Encoder::new(SAMPLE_RATE_HZ, Channels::Stereo, Application::Audio)?;
        encoder.set_bitrate(Bitrate::Bits(BITRATE_BPS))?;
        encoder.set_vbr(true)?;
        Ok(Self {
            encoder,
            pending_samples: Vec::with_capacity(INTERLEAVED_SAMPLES_PER_FRAME),
        })
    }

    pub(super) fn push_chunk(
        &mut self,
        chunk: CapturedAudioChunk,
    ) -> anyhow::Result<Vec<EncodedOpusPacket>> {
        self.pending_samples.extend(
            chunk
                .samples
                .into_iter()
                .map(|sample| sample.clamp(-1.0, 1.0)),
        );
        let frame_count = self.pending_samples.len() / INTERLEAVED_SAMPLES_PER_FRAME;
        let mut packets = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            let frame = &self.pending_samples[..INTERLEAVED_SAMPLES_PER_FRAME];
            let mut payload = vec![0_u8; MAX_OPUS_PACKET_BYTES];
            let encoded = self.encoder.encode_float(frame, &mut payload)?;
            payload.truncate(encoded);
            packets.push(EncodedOpusPacket { payload });
            self.pending_samples.drain(..INTERLEAVED_SAMPLES_PER_FRAME);
        }
        Ok(packets)
    }

    #[cfg(test)]
    fn pending_sample_count(&self) -> usize {
        self.pending_samples.len()
    }
}

pub(super) fn captured_audio_chunk(
    sample_buffer: &CMSampleBuffer,
) -> Result<CapturedAudioChunk, String> {
    let format = unsafe { sample_buffer.format_description() }
        .ok_or_else(|| "ScreenCaptureKit audio sample has no format description".to_string())?;
    let asbd_ptr = unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format) };
    let asbd = unsafe { asbd_ptr.as_ref() }
        .ok_or_else(|| "ScreenCaptureKit audio sample has no stream format".to_string())?;
    validate_stream_format(asbd)?;

    let frames = usize::try_from(unsafe { sample_buffer.num_samples() })
        .map_err(|_| "ScreenCaptureKit audio sample count is negative".to_string())?;
    if frames == 0 {
        return Err("ScreenCaptureKit audio sample contains no frames".to_string());
    }

    let mut required_size = 0_usize;
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            &mut required_size,
            std::ptr::null_mut(),
            0,
            None,
            None,
            0,
            std::ptr::null_mut(),
        )
    };
    if required_size < size_of::<AudioBufferList>() {
        return Err(format!(
            "ScreenCaptureKit audio buffer-list size query failed: status={status} size={required_size}"
        ));
    }

    let word_size = size_of::<usize>();
    let word_count = required_size.div_ceil(word_size);
    let mut storage = vec![0_usize; word_count];
    let list_ptr = storage.as_mut_ptr().cast::<AudioBufferList>();
    let mut block_buffer_ptr = std::ptr::null_mut::<CMBlockBuffer>();
    debug_assert_eq!((list_ptr as usize) % AUDIO_BUFFER_LIST_ALIGNMENT, 0);
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            std::ptr::null_mut(),
            list_ptr,
            storage.len() * word_size,
            None,
            None,
            0,
            &mut block_buffer_ptr,
        )
    };
    if status != 0 {
        return Err(format!(
            "ScreenCaptureKit audio buffer-list extraction failed: status={status}"
        ));
    }
    let block_buffer_ptr = NonNull::new(block_buffer_ptr)
        .ok_or_else(|| "ScreenCaptureKit audio extraction returned no block buffer".to_string())?;
    // SAFETY: CoreMedia returns the block buffer at +1. Keep it alive until
    // every AudioBufferList-backed sample has been copied into the owned vec.
    let _block_buffer = unsafe { CFRetained::from_raw(block_buffer_ptr) };
    let list = unsafe { &*list_ptr };
    let buffers = unsafe { audio_buffers(list)? };
    let samples = copy_interleaved_samples(asbd, frames, buffers)?;
    Ok(CapturedAudioChunk { samples })
}

fn validate_stream_format(asbd: &AudioStreamBasicDescription) -> Result<(), String> {
    if asbd.mFormatID != kAudioFormatLinearPCM
        || asbd.mFormatFlags & kAudioFormatFlagIsFloat == 0
        || asbd.mFormatFlags & kAudioFormatFlagIsBigEndian != 0
        || asbd.mBitsPerChannel != 32
        || asbd.mChannelsPerFrame as usize != CHANNELS
        || (asbd.mSampleRate - SAMPLE_RATE_HZ as f64).abs() > f64::EPSILON
    {
        return Err(format!(
            "unsupported ScreenCaptureKit audio format: rate={} channels={} format=0x{:08x} flags=0x{:08x} bits={}",
            asbd.mSampleRate,
            asbd.mChannelsPerFrame,
            asbd.mFormatID,
            asbd.mFormatFlags,
            asbd.mBitsPerChannel
        ));
    }
    Ok(())
}

unsafe fn audio_buffers(list: &AudioBufferList) -> Result<&[AudioBuffer], String> {
    let count = usize::try_from(list.mNumberBuffers)
        .map_err(|_| "ScreenCaptureKit audio buffer count overflow".to_string())?;
    if count == 0 || count > CHANNELS {
        return Err(format!(
            "unexpected ScreenCaptureKit audio buffer count: {count}"
        ));
    }
    Ok(unsafe { slice::from_raw_parts(list.mBuffers.as_ptr(), count) })
}

fn copy_interleaved_samples(
    asbd: &AudioStreamBasicDescription,
    frames: usize,
    buffers: &[AudioBuffer],
) -> Result<Vec<f32>, String> {
    let non_interleaved = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;
    if non_interleaved {
        if buffers.len() != CHANNELS {
            return Err(format!(
                "non-interleaved ScreenCaptureKit audio requires {CHANNELS} buffers, got {}",
                buffers.len()
            ));
        }
        let channels = buffers
            .iter()
            .map(|buffer| audio_buffer_as_f32(buffer, frames))
            .collect::<Result<Vec<_>, _>>()?;
        let mut samples = Vec::with_capacity(frames * CHANNELS);
        for frame in 0..frames {
            for channel in &channels {
                samples.push(channel[frame]);
            }
        }
        return Ok(samples);
    }

    if buffers.len() != 1 {
        return Err(format!(
            "interleaved ScreenCaptureKit audio requires one buffer, got {}",
            buffers.len()
        ));
    }
    Ok(audio_buffer_as_f32(&buffers[0], frames * CHANNELS)?.to_vec())
}

fn audio_buffer_as_f32(buffer: &AudioBuffer, samples: usize) -> Result<&[f32], String> {
    let required_bytes = samples
        .checked_mul(size_of::<f32>())
        .ok_or_else(|| "ScreenCaptureKit audio sample size overflow".to_string())?;
    if buffer.mData.is_null() || (buffer.mDataByteSize as usize) < required_bytes {
        return Err(format!(
            "ScreenCaptureKit audio buffer is short: required={required_bytes} actual={}",
            buffer.mDataByteSize
        ));
    }
    if (buffer.mData as usize) % align_of::<f32>() != 0 {
        return Err("ScreenCaptureKit audio buffer is not f32-aligned".to_string());
    }
    Ok(unsafe { slice::from_raw_parts(buffer.mData.cast::<f32>(), samples) })
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
    fn opus_packetizer_accumulates_exact_twenty_millisecond_frames() {
        let mut encoder = OpusPacketizer::new().expect("Opus encoder");
        assert!(encoder
            .push_chunk(chunk(INTERLEAVED_SAMPLES_PER_FRAME / 2))
            .unwrap()
            .is_empty());
        let packets = encoder
            .push_chunk(chunk(INTERLEAVED_SAMPLES_PER_FRAME / 2))
            .unwrap();
        assert_eq!(packets.len(), 1);
        assert!(!packets[0].payload.is_empty());
        assert_eq!(encoder.pending_sample_count(), 0);
    }

    #[test]
    fn opus_packetizer_retains_only_one_partial_frame() {
        let mut encoder = OpusPacketizer::new().expect("Opus encoder");
        let packets = encoder
            .push_chunk(chunk(INTERLEAVED_SAMPLES_PER_FRAME * 3 + 17))
            .unwrap();
        assert_eq!(packets.len(), 3);
        assert_eq!(encoder.pending_sample_count(), 17);
    }
}
