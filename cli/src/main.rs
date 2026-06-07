//! `orttaai` — headless dictation CLI. Phase 0 ships a `demo` that runs the full
//! coordinator loop with mock backends on any OS; real capture/injection/hotkey
//! commands arrive in Phase 1.

use anyhow::Result;
use orttaai_core::audio::{AudioCapture, MockAudioCapture};
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

fn devices() -> Result<()> {
    let audio = MockAudioCapture::default();
    println!("Audio input devices (mock backend):");
    for device in audio.devices()? {
        println!("  - {}", device.0);
    }
    println!("\n(real cpal enumeration arrives in Phase 1)");
    Ok(())
}

fn info() {
    let settings = Settings::default();
    let injector = SystemTextInjector::new();
    let hotkey = SystemHotkeyManager::new();

    println!("Orttaai for Linux & Windows — Phase 0 scaffold\n");
    println!("  config path:  {:?}", Settings::config_path());
    println!("  model:        {}", settings.model_id);
    println!("  push-to-talk: {:?}", settings.push_to_talk);
    println!("  injector:     {}", injector.backend_name());
    println!("  hotkey:       {}", hotkey.backend_name());
    println!("\n(real backends are stubbed until Phase 1 — see docs/architecture.md)");
}

fn print_help() {
    println!(
        "orttaai — cross-platform voice keyboard (Linux & Windows)\n\n\
         USAGE:\n  orttaai <COMMAND>\n\n\
         COMMANDS:\n\
         \x20 demo      Run the dictation loop with mock backends\n\
         \x20 devices   List audio input devices\n\
         \x20 info      Show config + selected platform backends\n\
         \x20 help      Show this help"
    );
}
