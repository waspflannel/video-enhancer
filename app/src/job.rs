use std::{io, path::PathBuf, sync::atomic::{AtomicBool, Ordering}};
use serde::{Deserialize, Serialize};
use crate::{parser::{Parser, FileData}, resolution::ResolutionEnhancer, frame_rate::{FrameRateEnhancer, FrameForEncoder}, video_adjuster::VideoAdjuster, sharpening::Sharpener, video_encoder::VideoEncoder, video_decoder};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnhancementSettings {
    pub denoise: f32,
    pub deblur: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub exposure: f32,
    pub warmth: f32,
    pub sharpening: f32,
}

impl Default for EnhancementSettings {
    fn default() -> Self {
        Self { denoise: 0.0, deblur: 0.0, contrast: 1.0, saturation: 1.0, vibrance: 0.0, exposure: 0.0, warmth: 0.0, sharpening: 0.0 }
    }
}

impl EnhancementSettings {
    pub fn presets() -> Vec<(&'static str, Self)> {
        vec![
            ("Neutral", Self::default()),
            ("Natural", Self { contrast: 1.04, vibrance: 0.08, sharpening: 0.15, ..Self::default() }),
            ("Crisp", Self { deblur: 0.2, contrast: 1.06, vibrance: 0.10, sharpening: 0.25, ..Self::default() }),
            ("Colour lift", Self { contrast: 1.08, vibrance: 0.20, saturation: 1.03, ..Self::default() }),
            ("Gentle cleanup", Self { denoise: 0.2, deblur: 0.15, ..Self::default() }),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VideoEnhancementJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub resolution_scale: u32,
    pub upscale_quality: u32,
    pub target_fps: Option<u32>,
    pub enhancements: EnhancementSettings,
}

impl VideoEnhancementJob {
    pub fn validate(&self, source: &FileData) -> io::Result<()> {
        if !(1..=4).contains(&self.resolution_scale) || !(1..=4).contains(&self.upscale_quality) {
            return Err(io::Error::other("Resolution scale and upscale quality must be between 1 and 4"));
        }
        let width = source.width.saturating_mul(self.resolution_scale);
        let height = source.height.saturating_mul(self.resolution_scale);
        if width > 4096 || height > 4096 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(io::Error::other("H.264 output requires even dimensions up to 4096 pixels per side. Choose a smaller scale."));
        }
        if let Some(fps) = self.target_fps && (![30, 60, 120].contains(&fps) || source.fps.is_some_and(|rate| f64::from(fps) < rate)) {
            return Err(io::Error::other("Choose Original FPS or an output rate at least as high as the source"));
        }
        let settings = &self.enhancements;
        for (name, value, low, high) in [
            ("Denoise", settings.denoise, 0.0, 1.0), ("Deblur", settings.deblur, 0.0, 1.0),
            ("Contrast", settings.contrast, 0.5, 1.5), ("Saturation", settings.saturation, 0.0, 2.0),
            ("Vibrance", settings.vibrance, -1.0, 1.0), ("Exposure", settings.exposure, -2.0, 2.0),
            ("Warmth", settings.warmth, -1.0, 1.0), ("Sharpening", settings.sharpening, 0.0, 2.0),
        ] {
            if !value.is_finite() || !(low..=high).contains(&value) { return Err(io::Error::other(format!("{name} is outside its supported range"))); }
        }
        if !self.output.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")) {
            return Err(io::Error::other("Choose an .mp4 output file"));
        }
        if self.output.exists() { return Err(io::Error::other("Output already exists. Choose a new filename; source and existing videos are never overwritten.")); }
        Ok(())
    }

    pub fn run(&self, cancelled: &AtomicBool, mut progress: impl FnMut(u64, f64)) -> io::Result<u64> {
        let source = Parser::new(&self.input).get_video_information()?;
        self.validate(&source)?;
        let mut resolution = ResolutionEnhancer::new(self);
        let mut frame_rate = FrameRateEnhancer::new(self, source.video_end_time)?;
        let mut adjuster = VideoAdjuster::new(self);
        let mut sharpener = Sharpener::new(self);
        let mut encoder = VideoEncoder::new(&source, self)?;
        let mut count = 0;
        let result = (|| {
            let mut encode = |timed: FrameForEncoder<'_>| {
                if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled")); }
                let adjusted = adjuster.adjust(timed.frame)?;
                let sharpened = sharpener.sharpen(adjusted)?;
                encoder.encode(FrameForEncoder { frame: sharpened, ..timed })?;
                count += 1;
                let seconds = timed.presentation_timestamp as f64 * timed.time_base.0 as f64 / timed.time_base.1 as f64;
                progress(count, seconds);
                Ok(())
            };
            video_decoder::decode(&source, |decoded| {
                if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled")); }
                let enhanced = resolution.enhance(&decoded)?;
                frame_rate.enhance(enhanced, &mut encode)
            })?;
            frame_rate.finish(&mut encode)?;
            encoder.finish()?;
            Ok(count)
        })();
        // An interrupted export is kept for diagnosis; it is never reported as complete.
        result.map_err(|error: io::Error| io::Error::new(error.kind(), format!("{error}. Incomplete output may remain at {}", self.output.display())))
    }
}
