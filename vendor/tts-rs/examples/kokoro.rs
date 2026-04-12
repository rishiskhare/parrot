use std::path::PathBuf;
use std::time::Instant;

use transcribe_rs::{
    engines::kokoro::{KokoroEngine, KokoroInferenceParams, KokoroModelParams},
    SynthesisEngine,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut engine = KokoroEngine::new();
    let model_path = PathBuf::from("models/kokoro");

    let load_start = Instant::now();
    engine.load_model_with_params(&model_path, KokoroModelParams::default())?;
    println!("Model loaded in {:.2?}", load_start.elapsed());

    println!("Available voices: {:?}", engine.list_voices());

    let text = "Hello! This is Kokoro, a text to speech model with multilingual support. \
                It supports American English, British English, French, Spanish, \
                Hindi, Italian, Japanese, Mandarin Chinese, and Brazilian Portuguese.";

    let params = KokoroInferenceParams {
        voice: "af_heart".to_string(),
        speed: 1.0,
        ..Default::default()
    };

    let synth_start = Instant::now();
    let result = engine.synthesize(text, Some(params))?;
    let synth_dur = synth_start.elapsed();

    let audio_duration = result.samples.len() as f64 / result.sample_rate as f64;
    let speedup = audio_duration / synth_dur.as_secs_f64();
    println!(
        "Synthesized {:.2}s audio in {:.2?} ({:.1}x real-time)",
        audio_duration, synth_dur, speedup
    );

    engine.synthesize_to_file(text, &PathBuf::from("output.wav"), None)?;
    println!("Saved to output.wav");

    engine.unload_model();
    Ok(())
}
