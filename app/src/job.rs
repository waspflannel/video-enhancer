use std::{io, path::PathBuf, sync::atomic::{AtomicBool, Ordering}};
use serde::{Deserialize, Serialize};
use crate::{parser::{Parser, FileData}, resolution::ResolutionEnhancer, frame_rate::{FrameRateEnhancer, FrameForEncoder}, video_adjuster::VideoAdjuster, sharpening::Sharpener, video_encoder::VideoEncoder, video_decoder};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnhancementSettings {
    pub denoise: f32,
    pub denoise_quality: u32,
    pub deblur: f32,
    pub deblur_quality: u32,
    pub contrast: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub exposure: f32,
    pub warmth: f32,
    pub sharpening: f32,
}

impl Default for EnhancementSettings {
    fn default() -> Self {
        Self { denoise: 0.0, denoise_quality: 0, deblur: 0.0, deblur_quality: 0, contrast: 1.0, saturation: 1.0, vibrance: 0.0, exposure: 0.0, warmth: 0.0, sharpening: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpscaleMethod { #[default] Vsr, Lightweight }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputEncoding { #[default] H264, Hevc10 }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PortraitMode { #[default] Off, Blur, Replace, Mask }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelightingMode { #[default] Off, Relighting, Aigs }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrameGenerationSettings {
    pub quality: u32,
    pub detect_scene_changes: bool,
}

impl Default for FrameGenerationSettings {
    fn default() -> Self { Self { quality: 1, detect_scene_changes: true } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EncoderSettings {
    pub preset: u32,
    pub quality: u32,
}

impl Default for EncoderSettings {
    fn default() -> Self { Self { preset: 4, quality: 19 } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HdrSettings {
    pub enabled: bool,
    pub contrast: u32,
    pub saturation: u32,
    pub middle_gray: u32,
    pub max_luminance: u32,
    pub debanding: bool,
}

impl Default for HdrSettings {
    fn default() -> Self { Self { enabled: false, contrast: 100, saturation: 100, middle_gray: 50, max_luminance: 650, debanding: true } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PortraitSettings {
    pub mode: PortraitMode,
    pub segmentation_mode: u32,
    pub blur_strength: f32,
    pub background_color: [u8; 3],
}

impl Default for PortraitSettings {
    fn default() -> Self { Self { mode: PortraitMode::Off, segmentation_mode: 0, blur_strength: 0.5, background_color: [0, 177, 64] } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelightingSettings {
    pub mode: RelightingMode,
    pub hdri: Option<PathBuf>,
    pub pan: f32,
    pub field_of_view: f32,
    pub foreground_gain: f32,
    pub background_gain: f32,
    pub environment_background: bool,
    pub specularity: f32,
    pub blur_strength: f32,
    pub quality: u32,
}

impl Default for RelightingSettings {
    fn default() -> Self {
        Self { mode: RelightingMode::Off, hdri: None, pan: 0.0, field_of_view: 60.0, foreground_gain: 1.0, background_gain: 1.0, environment_background: false, specularity: 0.0, blur_strength: 0.5, quality: 1 }
    }
}

fn full_strength() -> f32 { 1.0 }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VideoEnhancementJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub resolution_scale: f64,
    pub upscale_quality: u32,
    #[serde(default)]
    pub upscale_method: UpscaleMethod,
    #[serde(default = "full_strength")]
    pub upscale_strength: f32,
    pub target_fps: Option<u32>,
    pub enhancements: EnhancementSettings,
    #[serde(default)]
    pub frame_generation: FrameGenerationSettings,
    #[serde(default)]
    pub temporal_denoise: Option<u32>,
    #[serde(default)]
    pub portrait: PortraitSettings,
    #[serde(default)]
    pub relighting: RelightingSettings,
    #[serde(default)]
    pub output_encoding: OutputEncoding,
    #[serde(default)]
    pub encoder: EncoderSettings,
    #[serde(default)]
    pub hdr: HdrSettings,
}

impl VideoEnhancementJob {
    fn validate(&self, source: &FileData) -> io::Result<()> {
        if ![1.0, 4.0 / 3.0, 1.5, 2.0, 3.0, 4.0].contains(&self.resolution_scale) || ![0, 1, 2, 3, 4, 16, 17, 18, 19, 21, 23].contains(&self.upscale_quality) {
            return Err(io::Error::other("Choose a supported resolution scale and VSR quality mode"));
        }
        let width = (f64::from(source.width) * self.resolution_scale).round() as u32;
        let height = (f64::from(source.height) * self.resolution_scale).round() as u32;
        if width > 4096 || height > 4096 || !width.is_multiple_of(2) || !height.is_multiple_of(2) || u64::from(width) * u64::from(source.height) != u64::from(height) * u64::from(source.width) {
            return Err(io::Error::other("Choose a scale that preserves aspect ratio with even dimensions up to 4096 pixels per side."));
        }
        if let Some(fps) = self.target_fps && (![30, 60, 120].contains(&fps) || source.fps.is_some_and(|rate| f64::from(fps) < rate)) {
            return Err(io::Error::other("Choose Original FPS or an output rate at least as high as the source"));
        }
        let settings = &self.enhancements;
        if settings.denoise_quality > 3 || settings.deblur_quality > 3 || self.frame_generation.quality > 2 || self.temporal_denoise.is_some_and(|mode| mode > 1) || self.portrait.segmentation_mode > 3 || self.relighting.quality > 3 {
            return Err(io::Error::other("Choose a supported cleanup, portrait, relighting or frame generation quality"));
        }
        if !(1..=7).contains(&self.encoder.preset) || self.encoder.quality > 51 {
            return Err(io::Error::other("Encoder preset must be 1–7 and quality must be 0–51"));
        }
        if self.hdr.contrast > 200 || self.hdr.saturation > 200 || !(10..=100).contains(&self.hdr.middle_gray) || !(400..=2000).contains(&self.hdr.max_luminance) {
            return Err(io::Error::other("TrueHDR settings are outside the SDK's supported ranges"));
        }
        if self.hdr.enabled && self.output_encoding != OutputEncoding::Hevc10 {
            return Err(io::Error::other("TrueHDR requires HEVC 10-bit output"));
        }
        if self.output_encoding == OutputEncoding::Hevc10 && !self.hdr.enabled && (self.temporal_denoise.is_some() || self.portrait.mode != PortraitMode::Off || self.relighting.mode != RelightingMode::Off || (self.upscale_method == UpscaleMethod::Lightweight && self.resolution_scale > 1.0) || settings.sharpening != 0.0) {
            return Err(io::Error::other("10-bit SDR supports VSR cleanup/upscaling, colour controls and frame generation. Temporal denoise, portrait effects, lightweight upscale and SDK sharpening require 8-bit processing."));
        }
        if self.temporal_denoise.is_some() && (source.height < 80 || source.height > 1080 || source.width > 1920) {
            return Err(io::Error::other("NVIDIA temporal denoise supports source heights from 80 to 1080 pixels and widths up to 1920 pixels"));
        }
        if (self.portrait.mode != PortraitMode::Off || self.relighting.mode != RelightingMode::Off) && (source.width < 512 || source.height < 288) {
            return Err(io::Error::other("NVIDIA portrait effects require source video at least 512 × 288"));
        }
        if self.relighting.mode != RelightingMode::Off {
            if !self.relighting.hdri.as_ref().is_some_and(|path| path.is_file()) {
                return Err(io::Error::other("Choose an HDR environment image for relighting"));
            }
            if self.relighting.environment_background && self.portrait.mode != PortraitMode::Off {
                return Err(io::Error::other("Choose either an environment background or a portrait background effect"));
            }
            if self.relighting.mode == RelightingMode::Aigs && !self.relighting.environment_background {
                return Err(io::Error::other("Combined relighting uses the HDR environment as its background. Choose Relighting to preserve the source background."));
            }
        }
        for (name, value, low, high) in [
            ("Upscale strength", self.upscale_strength, 0.0, 1.0),
            ("Denoise", settings.denoise, 0.0, 1.0), ("Deblur", settings.deblur, 0.0, 1.0),
            ("Contrast", settings.contrast, 0.5, 1.5), ("Saturation", settings.saturation, 0.0, 2.0),
            ("Vibrance", settings.vibrance, -1.0, 1.0), ("Exposure", settings.exposure, -2.0, 2.0),
            ("Warmth", settings.warmth, -1.0, 1.0), ("Sharpening", settings.sharpening, 0.0, 2.0),
            ("Background blur", self.portrait.blur_strength, 0.0, 1.0),
            ("Environment rotation", self.relighting.pan, -180.0, 180.0),
            ("Field of view", self.relighting.field_of_view, 1.0, 179.0),
            ("Foreground brightness", self.relighting.foreground_gain, 0.0, 4.0),
            ("Background brightness", self.relighting.background_gain, 0.0, 4.0),
            ("Specularity", self.relighting.specularity, 0.0, 1.0),
            ("Relighting background blur", self.relighting.blur_strength, 0.0, 2.0),
        ] {
            if !(low..=high).contains(&value) { return Err(io::Error::other(format!("{name} is outside its supported range"))); }
        }
        if !self.output.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")) {
            return Err(io::Error::other("Choose an .mp4 output file"));
        }
        Ok(())
    }

    pub fn run(&self, cancelled: &AtomicBool, mut progress: impl FnMut(u64, f64)) -> io::Result<u64> {
        let source = Parser::new(&self.input).get_video_information()?;
        self.validate(&source)?;
        let mut frame_rate_enhancer = FrameRateEnhancer::new(self, source.video_end_time);
        // Source effects drop before the frames transferred into VFG storage.
        let mut resolution_enhancer = ResolutionEnhancer::new(self);
        let mut video_adjuster = VideoAdjuster::new(self);
        let mut sharpener = Sharpener::new(self);
        let mut hdr_converter = crate::hdr::TrueHdr::new(self);
        let mut video_encoder = VideoEncoder::new(&source, self)?;
        let mut encoded_frames = 0;
        let result = (|| {
            let mut encode_frame = |timed_frame: FrameForEncoder<'_>| {
                if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled")); }
                let adjusted_frame = video_adjuster.adjust(timed_frame.frame)?;
                let sharpened_frame = sharpener.sharpen(adjusted_frame)?;
                let output_frame = hdr_converter.enhance(sharpened_frame)?;
                video_encoder.encode(FrameForEncoder { frame: output_frame, ..timed_frame })?;
                encoded_frames += 1;
                let timestamp_seconds = timed_frame.presentation_timestamp as f64 * timed_frame.time_base.0 as f64 / timed_frame.time_base.1 as f64;
                progress(encoded_frames, timestamp_seconds);
                Ok(())
            };
            video_decoder::decode(&source, |decoded_frame| {
                if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled")); }
                let enhanced_frame = resolution_enhancer.enhance(&decoded_frame)?;
                frame_rate_enhancer.enhance(enhanced_frame, &mut encode_frame)
            })?;
            frame_rate_enhancer.finish(&mut encode_frame)?;
            video_encoder.finish()?;
            Ok(encoded_frames)
        })();
        frame_rate_enhancer.release_effect();
        // An interrupted export is kept for diagnosis; it is never reported as complete.
        result.map_err(|error: io::Error| io::Error::new(error.kind(), format!("{error}. Incomplete output may remain at {}", self.output.display())))
    }
}
