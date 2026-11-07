//! Hardware detection (CPU, RAM, GPU, disk) and model-tier recommendation
//! (Mac `HardwareDetector`, adapted to PCs).
//!
//! The Mac tiers by Apple-silicon generation; on Linux/Windows what decides
//! whisper speed is cores + RAM on CPU, or an NVIDIA GPU when this build links
//! the CUDA backend. Probes are best-effort and never fail: a value that can't
//! be read is `0`/empty and the tier falls back conservatively.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareTier {
    /// < 4 GB RAM or < 2 cores.
    Entry,
    /// The common laptop.
    Standard,
    /// ≥ 16 GB RAM and ≥ 8 threads.
    Performance,
    /// NVIDIA GPU with ≥ 4 GB VRAM and a CUDA build.
    Gpu,
}

impl HardwareTier {
    pub fn label(self) -> &'static str {
        match self {
            HardwareTier::Entry => "Entry",
            HardwareTier::Standard => "Standard",
            HardwareTier::Performance => "Performance",
            HardwareTier::Gpu => "GPU accelerated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub cpu_model: String,
    pub logical_cores: u32,
    pub physical_cores: Option<u32>,
    pub ram_gb: u32,
    /// NVIDIA GPUs found via `nvidia-smi`.
    pub gpus: Vec<GpuInfo>,
    /// Which whisper backend this binary links: `cpu | cuda | vulkan`.
    pub gpu_backend: &'static str,
    /// Free space where models are stored.
    pub free_disk_gb: f64,
    pub tier: HardwareTier,
    pub tier_label: &'static str,
    pub recommended_model: String,
    /// A note for the UI, e.g. an NVIDIA GPU on a CPU-only build.
    pub hint: Option<String>,
}

/// The whisper backend compiled into this binary.
pub fn gpu_backend() -> &'static str {
    if cfg!(feature = "cuda") {
        "cuda"
    } else if cfg!(feature = "vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}

/// Tier from the measured numbers (pure; unit-tested).
pub fn determine_tier(
    ram_gb: u32,
    logical_cores: u32,
    best_vram_mb: Option<u64>,
    gpu_build: bool,
) -> HardwareTier {
    if gpu_build && best_vram_mb.is_some_and(|v| v >= 4096) {
        return HardwareTier::Gpu;
    }
    // Unknown RAM (0) is treated as a standard machine, not an entry one.
    if (ram_gb != 0 && ram_gb < 4) || logical_cores < 2 {
        return HardwareTier::Entry;
    }
    if ram_gb >= 16 && logical_cores >= 8 {
        return HardwareTier::Performance;
    }
    HardwareTier::Standard
}

/// The model to recommend for `tier` and the dictation language (English-only
/// checkpoints are more accurate for English; anything else needs multilingual).
pub fn recommended_model(tier: HardwareTier, language: &str) -> String {
    let english = language.trim().eq_ignore_ascii_case("en");
    match (tier, english) {
        (HardwareTier::Gpu, _) => "ggml-large-v3-turbo-q5_0",
        (HardwareTier::Performance, true) => "ggml-small.en-q5_1",
        (HardwareTier::Performance, false) => "ggml-small-q5_1",
        (HardwareTier::Standard, true) => "ggml-base.en-q5_1",
        (HardwareTier::Standard, false) => "ggml-base-q5_1",
        (HardwareTier::Entry, true) => "ggml-tiny.en-q5_1",
        (HardwareTier::Entry, false) => "ggml-tiny-q5_1",
    }
    .to_string()
}

/// Detect this computer. `storage` is the models folder (for free disk space).
pub fn detect(storage: &Path, language: &str) -> HardwareInfo {
    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let (cpu_model, physical_cores) = cpu_info();
    let ram_gb = total_ram_bytes()
        .map(|b| ((b as f64) / (1024.0 * 1024.0 * 1024.0)).round() as u32)
        .unwrap_or(0);
    let gpus = nvidia_gpus();
    let backend = gpu_backend();
    let best_vram = gpus.iter().map(|g| g.vram_mb).max();
    let tier = determine_tier(ram_gb, logical_cores, best_vram, backend == "cuda");
    let hint = if backend == "cpu" && !gpus.is_empty() {
        Some(format!(
            "{} detected. The NVIDIA (CUDA) download of Orttaai transcribes several times faster.",
            gpus[0].name
        ))
    } else if backend == "cuda" && gpus.is_empty() {
        Some(
            "This is the NVIDIA build, but no NVIDIA GPU answered — speech runs on the CPU.".into(),
        )
    } else {
        None
    };
    HardwareInfo {
        cpu_model,
        logical_cores,
        physical_cores,
        ram_gb,
        gpus,
        gpu_backend: backend,
        free_disk_gb: free_disk_bytes(storage)
            .map(|b| b as f64 / 1_073_741_824.0)
            .unwrap_or(0.0),
        tier,
        tier_label: tier.label(),
        recommended_model: recommended_model(tier, language),
        hint,
    }
}

// ---- Parsers (pure) -----------------------------------------------------------

/// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits`.
pub fn parse_nvidia_smi(out: &str) -> Vec<GpuInfo> {
    out.lines()
        .filter_map(|line| {
            let (name, mem) = line.rsplit_once(',')?;
            let name = name.trim();
            let vram_mb = mem.trim().parse::<u64>().ok()?;
            (!name.is_empty()).then(|| GpuInfo {
                name: name.to_string(),
                vram_mb,
            })
        })
        .collect()
}

/// `MemTotal:` from `/proc/meminfo`, in bytes.
pub fn parse_meminfo(s: &str) -> Option<u64> {
    s.lines().find_map(|l| {
        let rest = l.strip_prefix("MemTotal:")?;
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kb * 1024)
    })
}

/// CPU model name and physical core count from `/proc/cpuinfo`.
pub fn parse_cpuinfo(s: &str) -> (String, Option<u32>) {
    let mut model = String::new();
    let mut cores = std::collections::HashSet::new();
    let mut physical = String::new();
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "model name" | "Model" | "Hardware" if model.is_empty() => model = v.to_string(),
            "physical id" => physical = v.to_string(),
            "core id" => {
                cores.insert(format!("{physical}:{v}"));
            }
            _ => {}
        }
    }
    let n = (!cores.is_empty()).then_some(cores.len() as u32);
    (model, n)
}

/// Available KiB (4th column) from `df -Pk <path>`.
pub fn parse_df(out: &str) -> Option<u64> {
    let line = out.lines().nth(1)?;
    let avail: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail * 1024)
}

/// `ProcessorNameString    REG_SZ    <name>` from `reg query`.
pub fn parse_reg_processor(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let idx = l.find("REG_SZ")?;
        let v = l[idx + "REG_SZ".len()..].trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

// ---- Probes -------------------------------------------------------------------

/// Run a helper command without flashing a console window on Windows.
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn nvidia_gpus() -> Vec<GpuInfo> {
    command_output(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|o| parse_nvidia_smi(&o))
    .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn cpu_info() -> (String, Option<u32>) {
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|s| parse_cpuinfo(&s))
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn cpu_info() -> (String, Option<u32>) {
    let name = command_output(
        "reg",
        &[
            "query",
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "/v",
            "ProcessorNameString",
        ],
    )
    .and_then(|o| parse_reg_processor(&o))
    .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
    .unwrap_or_default();
    (name, None)
}

#[cfg(target_os = "macos")]
fn cpu_info() -> (String, Option<u32>) {
    let name = command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let phys =
        command_output("sysctl", &["-n", "hw.physicalcpu"]).and_then(|s| s.trim().parse().ok());
    (name, phys)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn cpu_info() -> (String, Option<u32>) {
    (String::new(), None)
}

#[cfg(target_os = "linux")]
fn total_ram_bytes() -> Option<u64> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "macos")]
fn total_ram_bytes() -> Option<u64> {
    command_output("sysctl", &["-n", "hw.memsize"])?
        .trim()
        .parse()
        .ok()
}

#[cfg(target_os = "windows")]
fn total_ram_bytes() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn total_ram_bytes() -> Option<u64> {
    None
}

/// Free bytes on the volume holding `path` (the nearest existing ancestor).
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    free_disk_impl(&p)
}

#[cfg(unix)]
fn free_disk_impl(path: &Path) -> Option<u64> {
    parse_df(&command_output("df", &["-Pk", path.to_str()?])?)
}

#[cfg(windows)]
fn free_disk_impl(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free: u64 = 0;
    let dir = HSTRING::from(path.as_os_str());
    unsafe { GetDiskFreeSpaceExW(&dir, Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(not(any(unix, windows)))]
fn free_disk_impl(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers() {
        assert_eq!(determine_tier(32, 16, Some(8192), true), HardwareTier::Gpu);
        assert_eq!(
            determine_tier(32, 16, Some(8192), false),
            HardwareTier::Performance
        );
        assert_eq!(
            determine_tier(32, 16, Some(2048), true),
            HardwareTier::Performance
        );
        assert_eq!(determine_tier(8, 4, None, false), HardwareTier::Standard);
        assert_eq!(determine_tier(16, 4, None, false), HardwareTier::Standard);
        assert_eq!(determine_tier(2, 4, None, false), HardwareTier::Entry);
        assert_eq!(determine_tier(0, 4, None, false), HardwareTier::Standard);
    }

    #[test]
    fn recommendations_follow_language() {
        assert_eq!(
            recommended_model(HardwareTier::Standard, "en"),
            "ggml-base.en-q5_1"
        );
        assert_eq!(
            recommended_model(HardwareTier::Standard, "fr"),
            "ggml-base-q5_1"
        );
        assert_eq!(
            recommended_model(HardwareTier::Performance, "auto"),
            "ggml-small-q5_1"
        );
        assert_eq!(
            recommended_model(HardwareTier::Gpu, "en"),
            "ggml-large-v3-turbo-q5_0"
        );
    }

    #[cfg(feature = "models")]
    #[test]
    fn every_recommendation_is_in_the_catalog() {
        for tier in [
            HardwareTier::Entry,
            HardwareTier::Standard,
            HardwareTier::Performance,
            HardwareTier::Gpu,
        ] {
            for lang in ["en", "de"] {
                let id = recommended_model(tier, lang);
                assert!(crate::models::find_spec(&id).is_some(), "{id}");
            }
        }
    }

    #[test]
    fn parses_nvidia_smi() {
        let g = parse_nvidia_smi("NVIDIA GeForce RTX 4070, 12282\nTesla T4, 15360\nbad line\n");
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].name, "NVIDIA GeForce RTX 4070");
        assert_eq!(g[1].vram_mb, 15360);
    }

    #[test]
    fn parses_proc_files_and_df_and_reg() {
        assert_eq!(
            parse_meminfo("MemTotal:       16303428 kB\nMemFree: 1 kB"),
            Some(16303428 * 1024)
        );
        let cpu = "processor\t: 0\nmodel name\t: AMD Ryzen 7 5800X\nphysical id\t: 0\ncore id\t: 0\n\nprocessor\t: 1\nmodel name\t: AMD Ryzen 7 5800X\nphysical id\t: 0\ncore id\t: 0\n\nprocessor\t: 2\nphysical id\t: 0\ncore id\t: 1\n";
        assert_eq!(
            parse_cpuinfo(cpu),
            ("AMD Ryzen 7 5800X".to_string(), Some(2))
        );
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100 50 42 50% /\n";
        assert_eq!(parse_df(df), Some(42 * 1024));
        let reg = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\r\n    ProcessorNameString    REG_SZ    13th Gen Intel(R) Core(TM) i7-1360P\r\n";
        assert_eq!(
            parse_reg_processor(reg).as_deref(),
            Some("13th Gen Intel(R) Core(TM) i7-1360P")
        );
    }

    #[test]
    fn detect_never_panics() {
        let info = detect(&std::env::temp_dir(), "en");
        assert!(info.logical_cores >= 1);
        assert!(!info.recommended_model.is_empty());
    }
}
