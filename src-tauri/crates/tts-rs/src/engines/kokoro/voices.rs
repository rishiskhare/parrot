use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::model::KokoroError;

/// Storage for all loaded voice style vectors.
///
/// Each voice is stored as a flat list of style vectors, where each vector
/// has 256 floats. The index into the list corresponds to the phoneme token
/// count, enabling prosody-consistent synthesis.
pub struct VoiceStore {
    voices: HashMap<String, Vec<[f32; 256]>>,
}

impl VoiceStore {
    /// Load all voices from a .npz (numpy zip) file.
    ///
    /// The file should be a standard .npz archive where each entry is a
    /// .npy file named after the voice (e.g., `af_heart.npy`).
    pub fn load(path: &Path) -> Result<Self, KokoroError> {
        let file = File::open(path)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| KokoroError::VoiceParse(format!("Failed to open zip archive: {e}")))?;

        let mut voices = HashMap::new();

        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| {
                KokoroError::VoiceParse(format!("Failed to read zip entry {i}: {e}"))
            })?;

            let raw_name = entry.name().to_string();
            // Voice name is the entry name without the .npy extension
            let voice_name = raw_name
                .trim_end_matches('/')
                .trim_end_matches(".npy")
                .to_string();

            if voice_name.is_empty() || raw_name.ends_with('/') {
                continue;
            }

            let mut data = Vec::new();
            entry
                .read_to_end(&mut data)
                .map_err(|e| KokoroError::VoiceParse(format!("Failed to read {raw_name}: {e}")))?;

            let style_vectors = parse_npy(&data, &raw_name)?;
            voices.insert(voice_name, style_vectors);
        }

        log::info!("Loaded {} voices", voices.len());
        Ok(Self { voices })
    }

    /// Get the style vector for a voice at the given index.
    ///
    /// The index is clamped to the valid range, so any index is safe.
    pub fn get_style(&self, voice: &str, idx: usize) -> Result<[f32; 256], KokoroError> {
        let styles = self
            .voices
            .get(voice)
            .ok_or_else(|| KokoroError::VoiceNotFound(voice.to_string()))?;

        let clamped = idx.min(styles.len().saturating_sub(1));
        Ok(styles[clamped])
    }

    /// List all available voice names in sorted order.
    pub fn list_voices(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.voices.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }
}

/// Parse a numpy .npy file into a list of style vectors.
///
/// Expects a 2D float32 array of shape `[N, 256]` in little-endian format.
fn parse_npy(data: &[u8], name: &str) -> Result<Vec<[f32; 256]>, KokoroError> {
    // Verify numpy magic bytes: \x93NUMPY
    if data.len() < 10 {
        return Err(KokoroError::VoiceParse(format!(
            "{name}: file too short ({} bytes)",
            data.len()
        )));
    }

    if &data[0..6] != b"\x93NUMPY" {
        return Err(KokoroError::VoiceParse(format!(
            "{name}: invalid numpy magic bytes"
        )));
    }

    // major version at [6], minor at [7], header_len at [8..10] (little-endian u16)
    let header_len = u16::from_le_bytes([data[8], data[9]]) as usize;
    let data_offset = 10 + header_len;

    if data.len() < data_offset {
        return Err(KokoroError::VoiceParse(format!(
            "{name}: header truncated (need {data_offset} bytes, got {})",
            data.len()
        )));
    }

    let float_data = &data[data_offset..];
    if !float_data.len().is_multiple_of(4) {
        return Err(KokoroError::VoiceParse(format!(
            "{name}: float data length {} is not a multiple of 4",
            float_data.len()
        )));
    }

    let n_floats = float_data.len() / 4;
    if !n_floats.is_multiple_of(256) {
        return Err(KokoroError::VoiceParse(format!(
            "{name}: float count {n_floats} is not a multiple of 256 (style vector dim)"
        )));
    }

    let n_styles = n_floats / 256;
    let mut result = Vec::with_capacity(n_styles);

    for i in 0..n_styles {
        let mut vec = [0f32; 256];
        for (j, slot) in vec.iter_mut().enumerate() {
            let offset = (i * 256 + j) * 4;
            *slot = f32::from_le_bytes([
                float_data[offset],
                float_data[offset + 1],
                float_data[offset + 2],
                float_data[offset + 3],
            ]);
        }
        result.push(vec);
    }

    Ok(result)
}
