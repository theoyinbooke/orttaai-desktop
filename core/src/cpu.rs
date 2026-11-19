//! The CPU's cores, to size speech decoding to them, and Windows' background
//! throttling, which would slow decoding down while another app is in front.
//!
//! Speech models are compute-bound: every layer waits for its slowest thread.
//! On a hybrid CPU (Intel 12th gen and later: fast performance cores plus slow
//! efficiency cores) a thread per logical CPU puts most of the work on the
//! slow cores, and hyperthreads add little to matrix maths. So decoding uses
//! the physical performance cores, with a floor of four on CPUs that have them,
//! since a laptop chip with two fast cores still does better with two slow
//! ones helping.

use std::sync::OnceLock;

/// Physical cores: the fast ones and all of them. On a CPU with one kind of
/// core the two are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreCounts {
    pub performance: usize,
    pub total: usize,
}

/// This computer's cores, read once. `None` when the topology can't be read.
pub fn core_counts() -> Option<CoreCounts> {
    static COUNTS: OnceLock<Option<CoreCounts>> = OnceLock::new();
    *COUNTS.get_or_init(detect)
}

/// Threads for one speech decode on this computer.
pub fn speech_threads() -> i32 {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    recommended_threads(core_counts(), logical) as i32
}

/// The fast physical cores, at least four when there are that many physical
/// cores, at most eight and never more than the logical CPUs. Unknown
/// topology keeps the earlier rule: the logical CPUs, at most eight.
pub fn recommended_threads(counts: Option<CoreCounts>, logical: usize) -> usize {
    let logical = logical.max(1);
    let wanted = match counts {
        Some(c) if c.total > 0 => c.performance.max(c.total.min(4)),
        _ => logical,
    };
    wanted.clamp(1, 8).min(logical)
}

/// Threads for Word Boost's model, which scores the audio at the same time as
/// the speech model decodes it. It is about a sixth of the size, so a third
/// of the threads keeps it from finishing last without the two crowding the
/// same cores.
pub fn word_boost_threads(speech_threads: i32) -> i32 {
    ((speech_threads.max(1) + 2) / 3).max(1)
}

/// Asks Windows never to slow Orttaai down as a background app ("EcoQoS").
/// Dictation runs while another app is in front, which Windows 11 otherwise
/// treats as background work and may move to efficiency cores at low clock
/// speeds. Returns whether the request was accepted; elsewhere it does
/// nothing and returns `false`.
pub fn keep_full_speed_in_background() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
            PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            PROCESS_POWER_THROTTLING_STATE,
        };
        // Control the execution-speed policy (ControlMask) and turn it off
        // (StateMask 0): Windows then never throttles this process's speed.
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        // SAFETY: a valid, correctly sized struct for this information class,
        // on this process's own pseudo-handle.
        unsafe {
            SetProcessInformation(
                GetCurrentProcess(),
                ProcessPowerThrottling,
                &state as *const PROCESS_POWER_THROTTLING_STATE as *const core::ffi::c_void,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        }
        .is_ok()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn detect() -> Option<CoreCounts> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    let mut len = 0u32;
    // SAFETY: a size query with no buffer; it fails with the size it needs.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    if len == 0 {
        return None;
    }
    // u64 words keep the records 8-byte aligned.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    let base = buffer.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
    // SAFETY: `buffer` holds at least `len` bytes.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(base), &mut len) }
        .ok()?;
    let mut classes = Vec::new();
    let mut offset = 0usize;
    while offset < len as usize {
        // SAFETY: each record lies within the `len` bytes Windows wrote, at an
        // offset made of the records' own sizes.
        let record = unsafe {
            std::ptr::read_unaligned(
                (base as *const u8).add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX
            )
        };
        if record.Size == 0 {
            break;
        }
        if record.Relationship == RelationProcessorCore {
            // SAFETY: a processor-core record holds the `Processor` variant.
            classes.push(unsafe { record.Anonymous.Processor.EfficiencyClass });
        }
        offset += record.Size as usize;
    }
    counts_from_classes(&classes)
}

/// Physical cores from each core's efficiency class (Windows): the fast cores
/// are those of the highest class.
#[cfg_attr(not(windows), allow(dead_code))]
fn counts_from_classes(classes: &[u8]) -> Option<CoreCounts> {
    let top = *classes.iter().max()?;
    Some(CoreCounts {
        performance: classes.iter().filter(|&&c| c == top).count(),
        total: classes.len(),
    })
}

#[cfg(target_os = "linux")]
fn detect() -> Option<CoreCounts> {
    use std::collections::HashSet;
    let core_of = |cpu: usize| -> Option<(String, String)> {
        let dir = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
        let read = |name: &str| std::fs::read_to_string(format!("{dir}/{name}")).ok();
        Some((
            read("physical_package_id")?.trim().to_string(),
            read("core_id")?.trim().to_string(),
        ))
    };
    let online = std::fs::read_to_string("/sys/devices/system/cpu/online").ok()?;
    let all: HashSet<_> = parse_cpu_list(&online)
        .into_iter()
        .filter_map(core_of)
        .collect();
    if all.is_empty() {
        return None;
    }
    // Intel hybrid CPUs list their performance cores' CPUs here.
    let performance = std::fs::read_to_string("/sys/devices/cpu_core/cpus")
        .ok()
        .map(|list| {
            parse_cpu_list(&list)
                .into_iter()
                .filter_map(core_of)
                .collect::<HashSet<_>>()
                .len()
        })
        .filter(|&n| n > 0)
        .unwrap_or(all.len());
    Some(CoreCounts {
        performance,
        total: all.len(),
    })
}

#[cfg(target_os = "macos")]
fn detect() -> Option<CoreCounts> {
    let read = |key: &str| -> Option<usize> {
        let out = std::process::Command::new("sysctl")
            .args(["-n", key])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    };
    let total = read("hw.physicalcpu")?;
    // Apple silicon: level 0 is the performance cores.
    let performance = read("hw.perflevel0.physicalcpu").unwrap_or(total);
    Some(CoreCounts { performance, total })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn detect() -> Option<CoreCounts> {
    None
}

/// CPUs in a Linux list such as `0-3,8,10-11`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_cpu_list(list: &str) -> Vec<usize> {
    list.trim()
        .split(',')
        .filter(|part| !part.is_empty())
        .flat_map(|part| {
            let (start, end) = part.split_once('-').unwrap_or((part, part));
            match (start.trim().parse::<usize>(), end.trim().parse::<usize>()) {
                (Ok(start), Ok(end)) if start <= end => (start..=end).collect(),
                _ => Vec::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cores(performance: usize, total: usize) -> Option<CoreCounts> {
        Some(CoreCounts { performance, total })
    }

    #[test]
    fn hybrid_laptops_use_their_fast_cores_with_a_floor_of_four() {
        // i5-1235U: 2 performance + 8 efficiency cores, 12 logical CPUs.
        assert_eq!(recommended_threads(cores(2, 10), 12), 4);
        // i7-13700H: 6 + 8, 20 logical.
        assert_eq!(recommended_threads(cores(6, 14), 20), 6);
        // i9-13900K: 8 + 16, 32 logical: capped at eight.
        assert_eq!(recommended_threads(cores(8, 24), 32), 8);
    }

    #[test]
    fn hyperthreads_are_not_counted() {
        // A 4-core, 8-thread Intel laptop: four, not eight.
        assert_eq!(recommended_threads(cores(4, 4), 8), 4);
        // A 6-core, 12-thread desktop.
        assert_eq!(recommended_threads(cores(6, 6), 12), 6);
    }

    #[test]
    fn small_and_unknown_cpus() {
        assert_eq!(recommended_threads(cores(2, 2), 4), 2);
        assert_eq!(recommended_threads(cores(1, 1), 1), 1);
        // Never more than the logical CPUs, never zero.
        assert_eq!(recommended_threads(cores(4, 4), 2), 2);
        assert_eq!(recommended_threads(cores(0, 0), 0), 1);
        // Unknown topology: the logical CPUs, at most eight, as before.
        assert_eq!(recommended_threads(None, 12), 8);
        assert_eq!(recommended_threads(None, 4), 4);
    }

    #[test]
    fn word_boost_takes_about_a_third() {
        assert_eq!(word_boost_threads(1), 1);
        assert_eq!(word_boost_threads(4), 2);
        assert_eq!(word_boost_threads(6), 2);
        assert_eq!(word_boost_threads(8), 3);
        assert_eq!(word_boost_threads(0), 1);
    }

    #[test]
    fn efficiency_classes_pick_out_the_fast_cores() {
        // Windows reports class 1 for performance cores, 0 for efficiency ones.
        let hybrid = [1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(counts_from_classes(&hybrid), cores(2, 10));
        // One kind of core: all are "fast".
        assert_eq!(counts_from_classes(&[0, 0, 0, 0]), cores(4, 4));
        assert_eq!(counts_from_classes(&[]), None);
    }

    #[test]
    fn cpu_lists_parse() {
        assert_eq!(parse_cpu_list("0-3,8,10-11\n"), vec![0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("5"), vec![5]);
        assert!(parse_cpu_list("").is_empty());
        assert!(parse_cpu_list("3-1").is_empty());
    }

    #[test]
    fn this_computer_has_a_sensible_count() {
        let threads = speech_threads();
        eprintln!("speech threads {threads}, cores {:?}", core_counts());
        assert!((1..=8).contains(&threads), "{threads}");
    }
}
