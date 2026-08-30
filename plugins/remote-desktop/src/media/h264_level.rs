// EasyNet CLI — negotiated H.264 sender limits
// ==============================================
//
// RFC 6184 offer parameters describe what the browser can receive. This
// module turns that capability into one closed encoder/capture constraint so
// SDP, resolution, frame rate, bitrate, and the emitted SPS cannot drift.

#[cfg(test)]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::CaptureResizeMode;
use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    ScreenCaptureOptions, VideoResolution,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::daemon::plugins::remote_desktop) enum H264Level {
    Level1_0,
    Level1B,
    Level1_1,
    Level1_2,
    Level1_3,
    Level2_0,
    Level2_1,
    Level2_2,
    Level3_0,
    Level3_1,
    Level3_2,
    Level4_0,
    Level4_1,
    Level4_2,
    Level5_0,
    Level5_1,
    Level5_2,
}

impl H264Level {
    pub(in crate::daemon::plugins::remote_desktop) fn from_profile_level_bytes(
        profile_iop: u8,
        level_idc: u8,
    ) -> Option<Self> {
        if level_idc == 11 && profile_iop & 0x10 != 0 {
            return Some(Self::Level1B);
        }
        Some(match level_idc {
            10 => Self::Level1_0,
            11 => Self::Level1_1,
            12 => Self::Level1_2,
            13 => Self::Level1_3,
            20 => Self::Level2_0,
            21 => Self::Level2_1,
            22 => Self::Level2_2,
            30 => Self::Level3_0,
            31 => Self::Level3_1,
            32 => Self::Level3_2,
            40 => Self::Level4_0,
            41 => Self::Level4_1,
            42 => Self::Level4_2,
            50 => Self::Level5_0,
            51 => Self::Level5_1,
            52 => Self::Level5_2,
            _ => return None,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::Level1_0 => "1.0",
            Self::Level1B => "1b",
            Self::Level1_1 => "1.1",
            Self::Level1_2 => "1.2",
            Self::Level1_3 => "1.3",
            Self::Level2_0 => "2.0",
            Self::Level2_1 => "2.1",
            Self::Level2_2 => "2.2",
            Self::Level3_0 => "3.0",
            Self::Level3_1 => "3.1",
            Self::Level3_2 => "3.2",
            Self::Level4_0 => "4.0",
            Self::Level4_1 => "4.1",
            Self::Level4_2 => "4.2",
            Self::Level5_0 => "5.0",
            Self::Level5_1 => "5.1",
            Self::Level5_2 => "5.2",
        }
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) const fn level_idc(self) -> u8 {
        match self {
            Self::Level1B => 9,
            Self::Level1_0 => 10,
            Self::Level1_1 => 11,
            Self::Level1_2 => 12,
            Self::Level1_3 => 13,
            Self::Level2_0 => 20,
            Self::Level2_1 => 21,
            Self::Level2_2 => 22,
            Self::Level3_0 => 30,
            Self::Level3_1 => 31,
            Self::Level3_2 => 32,
            Self::Level4_0 => 40,
            Self::Level4_1 => 41,
            Self::Level4_2 => 42,
            Self::Level5_0 => 50,
            Self::Level5_1 => 51,
            Self::Level5_2 => 52,
        }
    }

    const fn max_macroblocks_per_frame(self) -> u32 {
        match self {
            Self::Level1_0 | Self::Level1B => 99,
            Self::Level1_1 | Self::Level1_2 | Self::Level1_3 | Self::Level2_0 => 396,
            Self::Level2_1 => 792,
            Self::Level2_2 | Self::Level3_0 => 1_620,
            Self::Level3_1 => 3_600,
            Self::Level3_2 => 5_120,
            Self::Level4_0 | Self::Level4_1 => 8_192,
            Self::Level4_2 => 8_704,
            Self::Level5_0 => 22_080,
            Self::Level5_1 | Self::Level5_2 => 36_864,
        }
    }

    const fn max_macroblocks_per_second(self) -> u32 {
        match self {
            Self::Level1_0 | Self::Level1B => 1_485,
            Self::Level1_1 => 3_000,
            Self::Level1_2 => 6_000,
            Self::Level1_3 | Self::Level2_0 => 11_880,
            Self::Level2_1 => 19_800,
            Self::Level2_2 => 20_250,
            Self::Level3_0 => 40_500,
            Self::Level3_1 => 108_000,
            Self::Level3_2 => 216_000,
            Self::Level4_0 | Self::Level4_1 => 245_760,
            Self::Level4_2 => 522_240,
            Self::Level5_0 => 589_824,
            Self::Level5_1 => 983_040,
            Self::Level5_2 => 2_073_600,
        }
    }

    const fn max_bitrate_kbps(self) -> u32 {
        match self {
            Self::Level1_0 => 64,
            Self::Level1B => 128,
            Self::Level1_1 => 192,
            Self::Level1_2 => 384,
            Self::Level1_3 => 768,
            Self::Level2_0 => 2_000,
            Self::Level2_1 | Self::Level2_2 => 4_000,
            Self::Level3_0 => 10_000,
            Self::Level3_1 => 14_000,
            Self::Level3_2 | Self::Level4_0 => 20_000,
            Self::Level4_1 | Self::Level4_2 => 50_000,
            Self::Level5_0 => 135_000,
            Self::Level5_1 | Self::Level5_2 => 240_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct H264ReceiveLimits {
    level: H264Level,
    max_macroblocks_per_frame: u32,
    max_macroblocks_per_second: u32,
    max_bitrate_kbps: u32,
}

impl H264ReceiveLimits {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        level: H264Level,
        max_macroblocks_per_frame: Option<u32>,
        max_macroblocks_per_second: Option<u32>,
        max_bitrate_kbps: Option<u32>,
    ) -> anyhow::Result<Self> {
        let base_fs = level.max_macroblocks_per_frame();
        let base_mbps = level.max_macroblocks_per_second();
        let base_bitrate = level.max_bitrate_kbps();
        let max_fs = extension_at_least("max-fs", max_macroblocks_per_frame, base_fs)?;
        let max_mbps = extension_at_least("max-mbps", max_macroblocks_per_second, base_mbps)?;
        let max_bitrate = extension_at_least("max-br", max_bitrate_kbps, base_bitrate)?;
        Ok(Self {
            level,
            max_macroblocks_per_frame: max_fs,
            max_macroblocks_per_second: max_mbps,
            max_bitrate_kbps: max_bitrate,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn level(self) -> H264Level {
        self.level
    }

    pub(in crate::daemon::plugins::remote_desktop) fn constrain(
        self,
        requested: &ScreenCaptureOptions,
        requested_bitrate_kbps: u32,
    ) -> anyhow::Result<(ScreenCaptureOptions, u32)> {
        let requested_resolution = requested.resolution.ok_or_else(|| {
            anyhow::anyhow!(
                "direct WebRTC H.264 requires explicit capture bounds before codec negotiation"
            )
        })?;
        let resolution =
            fit_resolution_to_macroblocks(requested_resolution, self.max_macroblocks_per_frame);
        let frame_macroblocks = macroblocks(resolution.width, resolution.height).max(1);
        let max_fps = (self.max_macroblocks_per_second / frame_macroblocks).max(1);
        let mut options = requested.clone();
        options.resolution = Some(resolution);
        options.fps = requested.fps.min(max_fps);
        Ok((options, requested_bitrate_kbps.min(self.max_bitrate_kbps)))
    }
}

fn extension_at_least(name: &str, extension: Option<u32>, base: u32) -> anyhow::Result<u32> {
    match extension {
        Some(value) if value < base => {
            anyhow::bail!("H.264 {name}={value} is below the negotiated level minimum {base}")
        }
        Some(value) => Ok(value),
        None => Ok(base),
    }
}

fn macroblocks(width: u32, height: u32) -> u32 {
    width.div_ceil(16).saturating_mul(height.div_ceil(16))
}

fn fit_resolution_to_macroblocks(
    requested: VideoResolution,
    max_macroblocks: u32,
) -> VideoResolution {
    if macroblocks(requested.width, requested.height) <= max_macroblocks {
        return VideoResolution {
            width: requested.width & !1,
            height: requested.height & !1,
        };
    }

    let requested_width = requested.width.max(2);
    let requested_height = requested.height.max(2);
    let max_requested_mb_width = requested_width.div_ceil(16).min(max_macroblocks);
    let max_requested_mb_height = requested_height.div_ceil(16).min(max_macroblocks);
    let mut best_scale = 0.0_f64;
    for mb_width in 1..=max_requested_mb_width {
        let mb_height = (max_macroblocks / mb_width).min(max_requested_mb_height);
        if mb_height == 0 {
            continue;
        }
        let scale = (f64::from(mb_width * 16) / f64::from(requested_width))
            .min(f64::from(mb_height * 16) / f64::from(requested_height));
        best_scale = best_scale.max(scale);
    }
    let width = ((f64::from(requested_width) * best_scale).floor() as u32 & !1).max(2);
    let height = ((f64::from(requested_height) * best_scale).floor() as u32 & !1).max(2);
    VideoResolution { width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_3_1_constrains_1080p60_to_720p30_and_14mbps() {
        let limits = H264ReceiveLimits::new(H264Level::Level3_1, None, None, None).unwrap();
        let (options, bitrate) = limits
            .constrain(
                &ScreenCaptureOptions {
                    fps: 60,
                    resolution: Some(VideoResolution {
                        width: 1920,
                        height: 1080,
                    }),
                    resize_mode: CaptureResizeMode::FitWithin,
                    region: None,
                },
                50_000,
            )
            .unwrap();

        assert_eq!(
            options.resolution,
            Some(VideoResolution {
                width: 1280,
                height: 720,
            })
        );
        assert_eq!(options.fps, 30);
        assert_eq!(bitrate, 14_000);
    }

    #[test]
    fn explicit_receiver_extensions_expand_only_the_named_limits() {
        let limits = H264ReceiveLimits::new(
            H264Level::Level3_1,
            Some(8_192),
            Some(245_760),
            Some(20_000),
        )
        .unwrap();
        let (options, bitrate) = limits
            .constrain(
                &ScreenCaptureOptions {
                    fps: 60,
                    resolution: Some(VideoResolution {
                        width: 1920,
                        height: 1080,
                    }),
                    resize_mode: CaptureResizeMode::FitWithin,
                    region: None,
                },
                50_000,
            )
            .unwrap();

        assert_eq!(options.resolution.unwrap().width, 1920);
        assert_eq!(options.resolution.unwrap().height, 1080);
        assert_eq!(options.fps, 30);
        assert_eq!(bitrate, 20_000);
    }

    #[test]
    fn malformed_receiver_extension_cannot_reduce_the_level_contract() {
        let error = H264ReceiveLimits::new(H264Level::Level3_1, Some(100), None, None)
            .expect_err("max-fs below level minimum must fail")
            .to_string();
        assert!(error.contains("max-fs=100"));
    }
}
