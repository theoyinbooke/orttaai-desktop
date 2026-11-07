//! Real whisper.cpp + Silero VAD tests. Ignored by default (they need model
//! files); run them with the models present:
//!
//! ```text
//! ORTTAAI_TEST_MODEL=models/ggml-base.en-q5_1.bin \
//! ORTTAAI_TEST_WAV=models/hello.wav \
//! ORTTAAI_TEST_VAD=models/ggml-silero-v5.1.2.bin \
//!   cargo test -p orttaai-core --features whisper -- --ignored
//! ```
#![cfg(feature = "whisper")]

use orttaai_core::transcription::{warm_up, Transcriber, WhisperTranscriber};
use orttaai_core::types::DecodeOptions;
use orttaai_core::vad::{SileroVad, VoiceDetector, SILERO_CHUNK_SAMPLES};
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
    assert_eq!(spec.sample_rate, 16_000, "test WAV must be 16 kHz");
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
fn transcribes_sample_with_and_without_bias_prompt() {
    let model = std::env::var("ORTTAAI_TEST_MODEL").expect("set ORTTAAI_TEST_MODEL");
    let wav = std::env::var("ORTTAAI_TEST_WAV").expect("set ORTTAAI_TEST_WAV");
    let expect = std::env::var("ORTTAAI_TEST_EXPECT").unwrap_or_else(|_| "hello".into());

    let samples = read_wav_16k_mono(&wav);
    let transcriber = WhisperTranscriber::from_path(Path::new(&model)).expect("load model");
    warm_up(&transcriber, &DecodeOptions::default()).expect("warm-up decode");
    let text = transcriber
        .transcribe(&samples, &DecodeOptions::default())
        .expect("transcribe")
        .to_lowercase();
    assert!(
        text.contains(&expect),
        "transcript missing {expect:?}: {text:?}"
    );

    let biased = DecodeOptions {
        initial_prompt: Some("Orttaai, Kubernetes.".into()),
        ..Default::default()
    };
    let text = transcriber.transcribe(&samples, &biased).expect("biased");
    assert!(text.to_lowercase().contains(&expect), "{text:?}");
}

#[test]
#[ignore = "requires ORTTAAI_TEST_VAD + ORTTAAI_TEST_WAV"]
fn silero_hears_speech_but_not_silence() {
    let vad = std::env::var("ORTTAAI_TEST_VAD").expect("set ORTTAAI_TEST_VAD");
    let wav = std::env::var("ORTTAAI_TEST_WAV").expect("set ORTTAAI_TEST_WAV");
    let mut detector = SileroVad::load(Path::new(&vad)).expect("load VAD");
    let speech = read_wav_16k_mono(&wav);
    let verdicts = detector.speech(&[], &speech).expect("detect");
    assert_eq!(verdicts.len(), speech.len() / SILERO_CHUNK_SAMPLES);
    assert!(verdicts.iter().any(|v| *v), "no speech heard: {verdicts:?}");

    let silence = vec![0.0f32; SILERO_CHUNK_SAMPLES * 8];
    let quiet = detector.speech(&speech, &silence).expect("detect silence");
    assert!(
        quiet.iter().all(|v| !v),
        "heard speech in silence: {quiet:?}"
    );
}
