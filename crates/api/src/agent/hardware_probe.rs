//! What accelerators this host actually has (D-18, ADR-0032).
//!
//! The resolver in `harness::hardware` decides what may run; this decides what it is
//! deciding about. Kept apart so the decision stays testable with no GPU present,
//! which is the only reason the loop's failure-mode tests run on this dev box.
//!
//! # Order of authority
//!
//! 1. `TBOT_GPU` — explicit configuration. Always wins, including when it declares
//!    *no* accelerator, because an operator who has said what the machine is should
//!    not be argued with by a probe.
//! 2. `nvidia-smi` — detection.
//! 3. Nothing. Not an error: a host with no accelerator is a real deployment (the
//!    frontier tier needs none), and it resolves to a tier that refuses local work
//!    rather than to a failure.
//!
//! # Pooling is not inferred
//!
//! Two cards in a box are not 48 GB. They are 48 GB **only** over NVLink with a
//! runtime that pools them, and nothing in `nvidia-smi`'s device list says whether
//! that is true. So detection reports `pooled: false` and pooling must be declared
//! (`TBOT_GPU_POOLED=1`). Guessing the other way turns a startup refusal into an OOM
//! four hours into a session.

use harness::hardware::{Device, Hardware, MemoryTopology};

/// Probes the host.
pub async fn detect() -> Hardware {
    if let Some(hw) = from_env() {
        return hw;
    }
    match nvidia_smi().await {
        Some(out) => assemble(parse_nvidia_smi(&out), pooled_from_env(), host_memory_gb()),
        None => assemble(Vec::new(), false, host_memory_gb()),
    }
}

fn assemble(devices: Vec<Device>, pooled: bool, host_gb: u64) -> Hardware {
    if let Ok(total) = std::env::var("TBOT_UNIFIED_MEMORY_GB") {
        if let Ok(gb) = total.trim().parse::<u64>() {
            // A unified-memory host is not "devices plus RAM"; it is one pool, and
            // adding the two would double-count it.
            return harness::hardware::unified(gb);
        }
    }
    harness::hardware::discrete(devices, pooled, host_gb)
}

fn pooled_from_env() -> bool {
    matches!(
        std::env::var("TBOT_GPU_POOLED").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

fn host_memory_gb() -> u64 {
    std::env::var("TBOT_HOST_MEMORY_GB")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(64)
}

/// `TBOT_GPU="RTX 3090:24576:8.6,RTX 3090:24576:8.6"` — name, MiB, compute capability.
fn from_env() -> Option<Hardware> {
    let raw = std::env::var("TBOT_GPU").ok()?;
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return Some(assemble(Vec::new(), false, host_memory_gb()));
    }
    let devices: Vec<Device> = raw
        .split(',')
        .filter_map(|d| parse_device(d, ':'))
        .collect();
    Some(assemble(devices, pooled_from_env(), host_memory_gb()))
}

fn parse_device(spec: &str, sep: char) -> Option<Device> {
    let parts: Vec<&str> = spec.split(sep).map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    let mib: u64 = parts[1].parse().ok()?;
    Some(Device {
        name: parts[0].to_string(),
        memory_bytes: mib * 1024 * 1024,
        compute_capability: parts.get(2).and_then(|c| parse_capability(c)),
        // Optional fourth column. Absent from the `TBOT_GPU` form, which declares a
        // machine rather than measuring one, so an operator saying what the box is
        // is not also forced to say what happens to be free at that instant.
        free_bytes: parts
            .get(3)
            .and_then(|f| f.trim().parse::<u64>().ok())
            .map(|f| f * 1024 * 1024),
    })
}

fn parse_capability(s: &str) -> Option<(u32, u32)> {
    let (major, minor) = s.trim().split_once('.')?;
    Some((major.trim().parse().ok()?, minor.trim().parse().ok()?))
}

/// Parses `nvidia-smi --query-gpu=name,memory.total,compute_cap,memory.free --format=csv,noheader,nounits`.
///
/// The `memory.free` column is optional so a three-column recording still parses.
///
/// Pure, so the table below is a test rather than a thing someone has to own a 3090
/// to check.
#[must_use]
pub fn parse_nvidia_smi(out: &str) -> Vec<Device> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| parse_device(l, ','))
        .collect()
}

async fn nvidia_smi() -> Option<String> {
    let out = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,compute_cap,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A one-line description for the startup log and the run record.
#[must_use]
pub fn describe(hw: &Hardware) -> String {
    match &hw.topology {
        MemoryTopology::Unified { total_bytes } => {
            format!("unified memory, {} GiB", total_bytes / (1 << 30))
        }
        MemoryTopology::Discrete {
            devices, pooled, ..
        } if devices.is_empty() => {
            let _ = pooled;
            "no accelerator".to_string()
        }
        MemoryTopology::Discrete {
            devices, pooled, ..
        } => {
            let names: Vec<String> = devices
                .iter()
                .map(|d| match d.free_bytes {
                    // Both numbers, because the gap is the whole diagnostic: a box
                    // reporting "11 GiB, 2.0 free" explains a slow run at a glance,
                    // and "11 GiB" alone does not.
                    Some(free) => format!(
                        "{} ({} GiB, {:.1} free)",
                        d.name,
                        d.memory_bytes / (1 << 30),
                        free as f64 / (1u64 << 30) as f64
                    ),
                    None => format!("{} ({} GiB)", d.name, d.memory_bytes / (1 << 30)),
                })
                .collect();
            format!(
                "{}{}",
                names.join(" + "),
                if *pooled && devices.len() > 1 {
                    ", pooled"
                } else if devices.len() > 1 {
                    ", NOT pooled"
                } else {
                    ""
                }
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_3090S: &str = "\
NVIDIA GeForce RTX 3090, 24576, 8.6
NVIDIA GeForce RTX 3090, 24576, 8.6";

    #[test]
    fn nvidia_smi_output_becomes_devices() {
        let d = parse_nvidia_smi(TWO_3090S);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].memory_bytes, 24 * 1024 * 1024 * 1024);
        assert_eq!(d[0].compute_capability, Some((8, 6)));
        assert!(d[0].meets_target_architecture());
    }

    #[test]
    fn the_dev_box_is_recognised_and_does_not_meet_the_target() {
        let d = parse_nvidia_smi("NVIDIA GeForce GTX 1080 Ti, 11264, 6.1");
        assert_eq!(d.len(), 1);
        assert!(
            !d[0].meets_target_architecture(),
            "Pascal has no usable tensor cores; that is the whole reason it is a degraded tier"
        );
    }

    /// The refusal that keeps a startup error from becoming a four-hour OOM. Nothing
    /// in the device list says whether two cards are pooled, so detection must not
    /// claim they are.
    #[test]
    fn two_cards_are_not_one_big_card_unless_someone_says_so() {
        let hw = assemble(parse_nvidia_smi(TWO_3090S), false, 64);
        assert_eq!(
            hw.usable_model_bytes(),
            24 * 1024 * 1024 * 1024,
            "unpooled, the largest single device is the budget"
        );
        let pooled = assemble(parse_nvidia_smi(TWO_3090S), true, 64);
        assert_eq!(pooled.usable_model_bytes(), 48 * 1024 * 1024 * 1024);
    }

    #[test]
    fn the_reference_model_needs_the_second_card() {
        use harness::hardware::{fits, reference_model};
        let one = assemble(
            parse_nvidia_smi("NVIDIA GeForce RTX 3090, 24576, 8.6"),
            false,
            64,
        );
        assert!(
            fits(&one, &reference_model()).is_err(),
            "21 GiB plus headroom does not fit 24 GB, and saying so at startup is the point"
        );
        assert!(fits(
            &assemble(parse_nvidia_smi(TWO_3090S), true, 64),
            &reference_model()
        )
        .is_ok());
    }

    #[test]
    fn a_host_with_no_accelerator_is_a_deployment_not_an_error() {
        let hw = assemble(Vec::new(), false, 64);
        assert_eq!(hw.usable_model_bytes(), 0);
        assert_eq!(describe(&hw), "no accelerator");
    }

    #[test]
    fn garbage_lines_are_skipped_rather_than_panicking() {
        let d = parse_nvidia_smi("\nNo devices were found\nNVIDIA GeForce RTX 3090, 24576, 8.6\n");
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn the_description_says_out_loud_when_cards_are_not_pooled() {
        let hw = assemble(parse_nvidia_smi(TWO_3090S), false, 64);
        assert!(describe(&hw).contains("NOT pooled"));
    }
}
