//! Real whisper.cpp backend test. Ignored by default (needs a downloaded model);
//! run it with the model + WAV present:
//!
//! ```text
//! ORTTAAI_TEST_MODEL=models/ggml-tiny.en.bin \
//! ORTTAAI_TEST_WAV=models/jfk.wav \
//!   cargo test -p orttaai-core --features whisper -- --ignored
//! ```
#![cfg(feature = "whisper")]

use orttaai_core::transcription::{Transcriber, WhisperTranscriber};
use orttaai_core::types::DecodeOptions;
use std::path::Path;

fn read_wav_16k_mono(path: &str) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / max)
                .collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };
    if spec.channels > 1 {
        interleaved
            .chunks(spec.channels as usize)
            .map(|f| f.iter().sum::<f32>() / f.len() as f32)
            .collect()
    } else {
        interleaved
    }
}

#[test]
#[ignore = "requires ORTTAAI_TEST_MODEL + ORTTAAI_TEST_WAV"]
fn transcribes_jfk_sample() {
    let model = std::env::var("ORTTAAI_TEST_MODEL").expect("set ORTTAAI_TEST_MODEL");
    let wav = std::env::var("ORTTAAI_TEST_WAV").expect("set ORTTAAI_TEST_WAV");

    let samples = read_wav_16k_mono(&wav);
    let transcriber = WhisperTranscriber::from_path(Path::new(&model)).expect("load model");
    let text = transcriber
        .transcribe(&samples, &DecodeOptions::default())
        .expect("transcribe")
        .to_lowercase();

    assert!(
        text.contains("country"),
        "transcript did not contain expected word: {text:?}"
    );
}
