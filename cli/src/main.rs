//! `orttaai` — headless dictation CLI. Phase 0 ships a `demo` that runs the full
//! coordinator loop with mock backends on any OS; real capture/injection/hotkey
//! commands arrive in Phase 1.

use anyhow::Result;
use orttaai_core::audio::MockAudioCapture;
use orttaai_core::coordinator::DictationCoordinator;
use orttaai_core::hotkey::{HotkeyManager, SystemHotkeyManager};
use orttaai_core::injection::{MockTextInjector, SystemTextInjector, TextInjector};
use orttaai_core::memory::MemoryService;
use orttaai_core::settings::Settings;
use orttaai_core::transcription::MockTranscriber;
use orttaai_core::types::DecodeOptions;

fn main() -> Result<()> {
    init_tracing();
    let cmd = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    match cmd.as_str() {
        "demo" => demo(),
        "transcribe" => transcribe_cmd(),
        "record" => record_cmd(),
        "inject" => inject_cmd(),
        "devices" => devices(),
        "info" => {
            info();
            Ok(())
        }
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Run the dictation loop end-to-end with mock backends.
fn demo() -> Result<()> {
    println!("▶ Orttaai demo (mock backends — works on any OS)\n");

    let injector = MockTextInjector::new();
    let log = injector.log();

    let mut memory = MemoryService::new();
    memory.add_term("orttaai", "Orttaai");
    memory.add_snippet("sig", "— sent via Orttaai");

    let mut coord = DictationCoordinator::new(
        Box::new(MockTranscriber::new("hello from orttaai sig")),
        Box::new(MockAudioCapture::new(1.5)),
        Box::new(injector),
        memory,
        DecodeOptions::default(),
    );

    println!("  hotkey down → recording…   (state: {:?})", coord.state());
    coord.on_press()?;
    println!("  hotkey up   → transcribing… (state: {:?})", coord.state());
    let outcome = coord.on_release()?;

    println!();
    println!("  result:     {:?}", outcome.result);
    println!("  transcript: {:?}", outcome.transcript);
    println!("  injected:   {:?}", log.last());
    println!("\n  ✔ Memory applied: 'orttaai'→'Orttaai', snippet 'sig' expanded.");
    Ok(())
}

#[cfg(feature = "audio")]
fn devices() -> Result<()> {
    use orttaai_core::audio::{AudioCapture, CpalAudioCapture};
    let audio = CpalAudioCapture::new();
    println!("Audio input devices (cpal):");
    for device in audio.devices()? {
        println!("  - {}", device.0);
    }
    Ok(())
}

#[cfg(not(feature = "audio"))]
fn devices() -> Result<()> {
    use orttaai_core::audio::AudioCapture;
    let audio = MockAudioCapture::default();
    println!("Audio input devices (mock backend):");
    for device in audio.devices()? {
        println!("  - {}", device.0);
    }
    println!("\n(build with --features audio for real cpal enumeration)");
    Ok(())
}

fn info() {
    let settings = Settings::default();
    let injector = SystemTextInjector::new();
    let hotkey = SystemHotkeyManager::new();

    println!("Orttaai for Linux & Windows\n");
    println!("  config path:  {:?}", Settings::config_path());
    println!("  model:        {}", settings.model_id);
    println!("  push-to-talk: {:?}", settings.push_to_talk);
    println!("  injector:     {}", injector.backend_name());
    println!("  hotkey:       {}", hotkey.backend_name());
    println!("\n(backends not built into this binary show as stubs — see docs/architecture.md)");
}

/// Transcribe a WAV file with the real whisper.cpp backend.
#[cfg(feature = "whisper")]
fn transcribe_cmd() -> Result<()> {
    use anyhow::Context;
    use orttaai_core::transcription::{Transcriber, WhisperTranscriber};
    use orttaai_core::types::DecodeOptions;
    use std::path::Path;

    const USAGE: &str = "usage: orttaai transcribe <model.bin> <audio.wav>";
    let mut args = std::env::args().skip(2);
    let model = args.next().context(USAGE)?;
    let wav = args.next().context(USAGE)?;

    let samples = read_wav_16k_mono(&wav)?;
    eprintln!(
        "loaded {} samples ({:.1}s) from {wav}; loading model {model}…",
        samples.len(),
        samples.len() as f32 / orttaai_core::types::TARGET_SAMPLE_RATE as f32
    );

    let transcriber = WhisperTranscriber::from_path(Path::new(&model))?;
    let text = transcriber.transcribe(&samples, &DecodeOptions::default())?;
    println!("{text}");
    Ok(())
}

#[cfg(not(feature = "whisper"))]
fn transcribe_cmd() -> Result<()> {
    eprintln!(
        "`transcribe` needs the whisper backend. Rebuild with:\n  \
         cargo run -p orttaai-cli --features whisper -- transcribe <model.bin> <audio.wav>"
    );
    std::process::exit(2);
}

/// Read a WAV into 16 kHz mono `f32`. (Resampling for non-16 kHz inputs is the
/// Phase 1 audio task; for now we warn.)
#[cfg(feature = "whisper")]
fn read_wav_16k_mono(path: &str) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<_, _>>()?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()?,
    };

    let mono = if spec.channels > 1 {
        interleaved
            .chunks(spec.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    } else {
        interleaved
    };

    if spec.sample_rate != orttaai_core::types::TARGET_SAMPLE_RATE {
        eprintln!(
            "warning: WAV is {} Hz; the engine expects {} Hz — accuracy may suffer until resampling lands (Phase 1)",
            spec.sample_rate,
            orttaai_core::types::TARGET_SAMPLE_RATE
        );
    }
    Ok(mono)
}

/// Capture from the default mic for N seconds, then optionally transcribe.
#[cfg(feature = "audio")]
fn record_cmd() -> Result<()> {
    use anyhow::Context;
    use orttaai_core::audio::{AudioCapture, CpalAudioCapture};
    use orttaai_core::types::TARGET_SAMPLE_RATE;

    let mut args = std::env::args().skip(2);
    let seconds: f32 = args
        .next()
        .unwrap_or_else(|| "5".to_string())
        .parse()
        .context("usage: orttaai record <seconds> [model.bin]")?;
    let model = args.next();

    let mut capture = CpalAudioCapture::new();
    eprintln!(
        "recording {seconds:.0}s from the default mic… (grant microphone permission if prompted)"
    );
    capture.start(None)?;

    let ticks = ((seconds * 10.0) as u32).max(1);
    for _ in 0..ticks {
        std::thread::sleep(std::time::Duration::from_millis(100));
        eprint!("\r  level: {:>5.2}  ", capture.level());
    }
    eprintln!();

    let samples = capture.stop()?;
    eprintln!(
        "captured {} samples ({:.1}s @ {} Hz)",
        samples.len(),
        samples.len() as f32 / TARGET_SAMPLE_RATE as f32,
        TARGET_SAMPLE_RATE
    );

    match model {
        Some(model) => transcribe_samples(&model, &samples),
        None => {
            println!("(no model given — add one and build with --features \"audio whisper\" to transcribe)");
            Ok(())
        }
    }
}

#[cfg(not(feature = "audio"))]
fn record_cmd() -> Result<()> {
    eprintln!(
        "`record` needs the audio backend. Rebuild with:\n  \
         cargo run -p orttaai-cli --features \"audio whisper\" -- record <seconds> [model.bin]"
    );
    std::process::exit(2);
}

#[cfg(all(feature = "audio", feature = "whisper"))]
fn transcribe_samples(model: &str, samples: &[f32]) -> Result<()> {
    use orttaai_core::transcription::{Transcriber, WhisperTranscriber};
    use orttaai_core::types::DecodeOptions;
    let transcriber = WhisperTranscriber::from_path(std::path::Path::new(model))?;
    println!(
        "{}",
        transcriber.transcribe(samples, &DecodeOptions::default())?
    );
    Ok(())
}

#[cfg(all(feature = "audio", not(feature = "whisper")))]
fn transcribe_samples(_model: &str, _samples: &[f32]) -> Result<()> {
    eprintln!("a model was given, but transcription needs the whisper backend. Rebuild with --features \"audio whisper\".");
    Ok(())
}

/// Type a string into the focused window via the real injection backend.
#[cfg(feature = "injection")]
fn inject_cmd() -> Result<()> {
    use anyhow::Context;
    use orttaai_core::injection::{SystemTextInjector, TextInjector};
    use orttaai_core::types::InjectionResult;

    let text = std::env::args()
        .nth(2)
        .context("usage: orttaai inject <text>")?;
    let injector = SystemTextInjector::new();

    eprintln!(
        "injecting via {} in 2s — click into your target window now…",
        injector.backend_name()
    );
    std::thread::sleep(std::time::Duration::from_secs(2));

    match injector.inject(&text)? {
        InjectionResult::Success => eprintln!("✓ injected"),
        other => eprintln!("result: {other:?}"),
    }
    Ok(())
}

#[cfg(not(feature = "injection"))]
fn inject_cmd() -> Result<()> {
    eprintln!(
        "`inject` needs the injection backend. Rebuild with:\n  \
         cargo run -p orttaai-cli --features injection -- inject \"<text>\""
    );
    std::process::exit(2);
}

fn print_help() {
    println!(
        "orttaai — cross-platform voice keyboard (Linux & Windows)\n\n\
         USAGE:\n  orttaai <COMMAND>\n\n\
         COMMANDS:\n\
         \x20 demo                       Run the dictation loop with mock backends\n\
         \x20 record <secs> [model]      Capture from the mic, then transcribe (needs --features \"audio whisper\")\n\
         \x20 transcribe <model> <wav>   Transcribe a WAV (needs --features whisper)\n\
         \x20 inject <text>              Type text into the focused window (needs --features injection)\n\
         \x20 devices                    List audio input devices\n\
         \x20 info                       Show config + selected platform backends\n\
         \x20 help                       Show this help"
    );
}
