//! Pluggable TTS model families.
//!
//! To support a new kind of model, add a module implementing [`ModelFamily`]
//! (Hugging Face repo detection + loading into a [`SpeechEngine`]), add its
//! [`EngineType`] variant, and list it in [`FAMILIES`]. Download, playback,
//! history and the Models UI work unchanged.

mod kokoro;

use std::path::{Path, PathBuf};

use tts_rs::SynthesisResult;

use crate::managers::model::{
    hf_file_url, size_mb, EngineType, HfSibling, ModelComponent, ModelInfo,
};

/// A loaded model that can turn text into audio.
pub trait SpeechEngine: Send {
    fn list_voices(&self) -> Vec<String>;

    /// Voice used for warmup and when the configured voice is unavailable.
    fn default_voice(&self) -> String {
        self.list_voices().into_iter().next().unwrap_or_default()
    }

    fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        style_index: usize,
        speed: f32,
    ) -> Result<SynthesisResult, String>;
}

pub struct LoadContext<'a> {
    pub model_dir: &'a Path,
    /// Writable per-model directory for caches (e.g. optimized ONNX graphs).
    pub cache_dir: Option<PathBuf>,
    pub espeak_bin: Option<PathBuf>,
    pub espeak_data: Option<PathBuf>,
    pub num_threads: usize,
}

/// File listing of a Hugging Face model repository.
pub struct HfRepo<'a> {
    pub id: &'a str,
    pub files: &'a [HfSibling],
}

impl HfRepo<'_> {
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.rfilename.as_str())
    }

    pub fn size(&self, path: &str) -> Option<u64> {
        self.files.iter().find(|f| f.rfilename == path)?.size
    }

    /// Model ID prefix unique to this repo.
    pub fn slug(&self) -> String {
        format!("hf-{}", self.id.replace('/', "--"))
    }

    pub fn component(&self, path: &str, filename: String) -> ModelComponent {
        ModelComponent {
            url: hf_file_url(self.id, path),
            filename,
            size_mb: self.size(path).map(size_mb).unwrap_or(0),
        }
    }
}

pub trait ModelFamily: Sync {
    fn engine_type(&self) -> EngineType;

    /// Human-readable file layout, shown when a repo matches no family.
    fn layout(&self) -> &'static str;

    /// Models this family can run from the repo; empty if it doesn't match.
    fn detect(&self, repo: &HfRepo) -> Vec<ModelInfo>;

    fn load(&self, ctx: &LoadContext) -> Result<Box<dyn SpeechEngine>, String>;
}

/// Registered families, in detection priority order.
/// Piper is disabled until it has had more testing.
pub static FAMILIES: &[&dyn ModelFamily] = &[&kokoro::Kokoro];

pub fn family_for(engine_type: &EngineType) -> &'static dyn ModelFamily {
    *FAMILIES
        .iter()
        .find(|f| &f.engine_type() == engine_type)
        .expect("every EngineType has a registered ModelFamily")
}

pub fn detect(repo: &HfRepo) -> anyhow::Result<Vec<ModelInfo>> {
    for family in FAMILIES {
        let models = family.detect(repo);
        if !models.is_empty() {
            return Ok(models);
        }
    }
    let supported: Vec<_> = FAMILIES.iter().map(|f| f.layout()).collect();
    anyhow::bail!(
        "{} has no model Parrot can run. Supported: {}.",
        repo.id,
        supported.join(" and ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_type_has_a_family() {
        for engine_type in [EngineType::Kokoro] {
            assert_eq!(family_for(&engine_type).engine_type(), engine_type);
        }
    }
}
