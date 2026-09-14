//! Accelerator inventory and the capability resolver (D-18, ADR-0032).
//!
//! Two rules shape this module, and both come from the deployment target rather
//! than from whatever box the code happens to be compiled on:
//!
//! 1. **Never assume one accelerator.** The near-term target is two RTX 3090s over
//!    NVLink with 48 GB pooled. Code that reads "the GPU" is code that has to be
//!    rewritten the day the second card arrives, and it usually gets rewritten
//!    wrong — the single-device assumption hides in defaults, not in obvious places.
//!
//! 2. **Never assume device memory and host memory are distinct pools.** The stretch
//!    target is a 128 GB unified-memory host running 120B-class MoE models. On such a
//!    host, "VRAM" is not a separate budget, and any arithmetic that subtracts one
//!    from the other is wrong. [`MemoryTopology`] makes that a type-level question
//!    rather than an assumption.
//!
//! What this module deliberately does **not** do is accommodate the current dev box.
//! A GTX 1080 Ti with 11 GB of Pascal memory resolves to a degraded tier through the
//! ordinary path — it is not a special case, and there are no FP32 fallbacks or
//! 11 GB-tuned defaults anywhere in the core.

use serde::{Deserialize, Serialize};

/// One accelerator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// Total memory on this device, in bytes.
    pub memory_bytes: u64,
    /// CUDA compute capability as `(major, minor)`, when known.
    ///
    /// Ampere is `(8, 6)` for a 3090; Pascal is `(6, 1)` for a 1080 Ti. The target
    /// assumes Ampere or newer, which is what makes BF16 and flash-attention
    /// backends available rather than optional.
    pub compute_capability: Option<(u32, u32)>,
    /// Memory actually free right now, when the probe could measure it.
    ///
    /// On a headless server this is nearly `memory_bytes` and the distinction is
    /// academic. On a workstation it is not: a desktop session with a browser and a
    /// couple of Electron apps holds well over a gigabyte of VRAM, and measurement
    /// on the dev box found the gap swinging between 1.4 GB and 6 GB depending on
    /// what was open. Fitting against the total there is not optimism, it is a
    /// different number — and the failure it produces is the quiet one, because the
    /// backend does not refuse. It silently offloads layers to host memory and the
    /// model runs an order of magnitude slower, which reads as the agent hanging.
    ///
    /// `None` means "not measured", and the total is used, so every caller that
    /// never knew about this field keeps its old behaviour.
    #[serde(default)]
    pub free_bytes: Option<u64>,
}

impl Device {
    /// Whether this device has the tensor-core generation the target assumes.
    ///
    /// Ampere (8.0) and newer have BF16 and the memory bandwidth the flash-attention
    /// kernels are written against. Below that, a backend either falls back to
    /// slower paths or refuses — and the *fallback* is what must never leak into the
    /// core path, so this is a question the resolver asks once, up front.
    #[must_use]
    pub fn meets_target_architecture(&self) -> bool {
        matches!(self.compute_capability, Some((major, _)) if major >= 8)
    }

    /// The memory a model may actually claim on this device: what is free when that
    /// was measured, the total otherwise.
    #[must_use]
    pub fn available_bytes(&self) -> u64 {
        self.free_bytes.unwrap_or(self.memory_bytes)
    }
}

/// How memory is organised across the host and its accelerators.
///
/// The distinction exists because the fit calculation genuinely differs, and getting
/// it wrong in the unified case means refusing a model that would have run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryTopology {
    /// Device memory is separate from host memory. A model must fit in the
    /// accelerators (pooled, if they are linked).
    Discrete {
        devices: Vec<Device>,
        /// Whether the devices share an address space well enough to hold one model
        /// across them (NVLink, or a backend that shards). When false, the largest
        /// single device is the ceiling, not the sum.
        pooled: bool,
        host_memory_bytes: u64,
    },
    /// Host and device draw on one pool — Apple silicon, Grace-Hopper-class parts,
    /// and the 128 GB unified host in the stretch target. There is no "VRAM" to
    /// subtract from anything.
    Unified { total_bytes: u64 },
}

/// What the machine actually has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hardware {
    pub topology: MemoryTopology,
}

impl Hardware {
    /// The largest contiguous model this machine can hold, in bytes.
    ///
    /// For discrete pooled memory this is the sum; for discrete un-pooled it is the
    /// **largest single device**, because a model that does not fit on one card and
    /// cannot be sharded does not run just because the total looks sufficient. That
    /// distinction is the entire reason `pooled` is a field.
    #[must_use]
    pub fn usable_model_bytes(&self) -> u64 {
        match &self.topology {
            MemoryTopology::Unified { total_bytes } => *total_bytes,
            MemoryTopology::Discrete {
                devices, pooled, ..
            } => {
                if *pooled {
                    devices.iter().map(Device::available_bytes).sum()
                } else {
                    devices.iter().map(Device::available_bytes).max().unwrap_or(0)
                }
            }
        }
    }

    #[must_use]
    pub fn devices(&self) -> &[Device] {
        match &self.topology {
            MemoryTopology::Discrete { devices, .. } => devices,
            MemoryTopology::Unified { .. } => &[],
        }
    }

    /// Whether every accelerator meets the target architecture.
    ///
    /// Unified-memory hosts answer `true`: they are the stretch target, and their
    /// capability question is about bandwidth rather than tensor-core generation.
    #[must_use]
    pub fn meets_target_architecture(&self) -> bool {
        match &self.topology {
            MemoryTopology::Unified { .. } => true,
            MemoryTopology::Discrete { devices, .. } => {
                !devices.is_empty() && devices.iter().all(Device::meets_target_architecture)
            }
        }
    }

    #[must_use]
    pub fn accelerator_count(&self) -> usize {
        match &self.topology {
            MemoryTopology::Discrete { devices, .. } => devices.len(),
            MemoryTopology::Unified { .. } => 1,
        }
    }
}

/// How much headroom a model needs beyond its weights.
///
/// KV cache, activations and fragmentation. 20% is a working figure, not a measured
/// one; the resolver's job is to be *honest about the margin*, not to predict it
/// exactly. An eval on the target hardware replaces this.
const HEADROOM: f64 = 1.20;

/// What a model needs to run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequirements {
    /// Resident size of the weights at the deployed quantisation, in bytes.
    ///
    /// At the deployed quantisation specifically — guide §6.1 is explicit that
    /// tool-call reliability has to be verified at the quant level actually shipped,
    /// and the same applies to whether it fits.
    pub resident_bytes: u64,
    /// Whether this model needs Ampere-or-newer to run the way it is configured.
    pub requires_target_architecture: bool,
}

impl ModelRequirements {
    #[must_use]
    pub fn with_headroom(&self) -> u64 {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        {
            (self.resident_bytes as f64 * HEADROOM) as u64
        }
    }
}

/// Why a model cannot run here.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum Unfit {
    /// Not enough memory, pooled or otherwise.
    Memory { needs: u64, has: u64 },
    /// The accelerators are older than the model's configuration assumes.
    Architecture { detail: String },
    /// No accelerator at all.
    NoAccelerator,
}

impl Unfit {
    /// Operator-facing explanation. Says what to do, not only what is wrong.
    #[must_use]
    pub fn explain(&self) -> String {
        match self {
            Unfit::Memory { needs, has } => format!(
                "this model needs about {:.1} GB including headroom and this machine offers {:.1} GB; \
                 use a smaller quantisation, a smaller model, or add an accelerator",
                gb(*needs),
                gb(*has)
            ),
            Unfit::Architecture { detail } => format!(
                "{detail}; the local tier targets Ampere or newer, and older cards run \
                 as a degraded tier rather than on the main path"
            ),
            Unfit::NoAccelerator => {
                "no accelerator was found; the local tier needs one".to_string()
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}

/// Whether a model fits this machine.
pub fn fits(hw: &Hardware, req: &ModelRequirements) -> Result<(), Unfit> {
    if hw.accelerator_count() == 0 {
        return Err(Unfit::NoAccelerator);
    }
    if req.requires_target_architecture && !hw.meets_target_architecture() {
        let names: Vec<&str> = hw
            .devices()
            .iter()
            .filter(|d| !d.meets_target_architecture())
            .map(|d| d.name.as_str())
            .collect();
        return Err(Unfit::Architecture {
            detail: format!("{} is older than Ampere", names.join(", ")),
        });
    }
    let needs = req.with_headroom();
    let has = hw.usable_model_bytes();
    if needs > has {
        return Err(Unfit::Memory { needs, has });
    }
    Ok(())
}

// ── Tool-calling capability ──────────────────────────────────────────────────

/// How much tool use a resolved configuration can actually sustain.
///
/// This is the field that makes the degraded tier honest. A machine that cannot run
/// the reference model still runs *something*, and the temptation is to let the
/// agent carry on with a weaker model and hope. That produces a research session
/// that fails slowly and expensively, in ways that look like the agent being bad at
/// its job rather than the hardware being too small.
///
/// So a configuration declares what it can do, and a task that needs more is
/// **refused or escalated** — never silently downgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCalling {
    /// No reliable tool use. Useful for routing and classification inside the
    /// harness (guide §6.1 puts sub-7B models here), not for driving a task.
    None,
    /// One tool call per task, in isolation. Enough for "answer this question with
    /// one lookup"; not enough for a research session.
    SingleShot,
    /// Chains of calls with state carried across steps. The baseline assumption for
    /// the reference model on the target hardware.
    MultiStep,
}

impl ToolCalling {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ToolCalling::None => "none",
            ToolCalling::SingleShot => "single_shot",
            ToolCalling::MultiStep => "multi_step",
        }
    }
}

/// What a task needs from the model driving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskDemand {
    /// A question answered from one lookup.
    SingleLookup,
    /// A chain: read, compute, decide, report.
    MultiStep,
}

impl TaskDemand {
    #[must_use]
    pub fn required(self) -> ToolCalling {
        match self {
            TaskDemand::SingleLookup => ToolCalling::SingleShot,
            TaskDemand::MultiStep => ToolCalling::MultiStep,
        }
    }
}

/// What the harness does when a task outgrows its tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Admission {
    /// The configuration can run this task.
    Admit,
    /// It cannot, and a more capable tier is configured — hand it over.
    Escalate { reason: String, to: String },
    /// It cannot, and there is nowhere to send it. Say so plainly.
    Refuse { reason: String, fix: String },
}

/// Decides whether a task may run on a configuration (D-18).
///
/// `escalation_target` is the profile id of a more capable tier, when one is
/// configured. Escalation is preferred to refusal — a frontier profile sitting
/// unused while a local tier refuses work is a worse outcome than paying for the
/// frontier call — but refusing is strictly better than pretending.
#[must_use]
pub fn admit(
    capability: ToolCalling,
    demand: TaskDemand,
    escalation_target: Option<&str>,
) -> Admission {
    let needed = demand.required();
    if capability >= needed {
        return Admission::Admit;
    }
    let reason = format!(
        "this task needs {} tool calling and the resolved tier provides {}",
        needed.as_str(),
        capability.as_str()
    );
    match escalation_target {
        Some(to) => Admission::Escalate {
            reason,
            to: to.to_string(),
        },
        None => Admission::Refuse {
            reason,
            fix: "run this on hardware that fits the reference model, or configure a \
                  frontier profile to escalate to"
                .into(),
        },
    }
}

// ── Fixtures for the targets, so tests read as the hardware they describe ────

/// A single RTX 3090. The primary deployment target.
#[must_use]
pub fn rtx_3090() -> Device {
    Device {
        name: "NVIDIA GeForce RTX 3090".into(),
        memory_bytes: 24 * 1024 * 1024 * 1024,
        compute_capability: Some((8, 6)),
        free_bytes: None,
    }
}

/// A GTX 1080 Ti. The dev box, and the degraded tier's reference point.
#[must_use]
pub fn gtx_1080ti() -> Device {
    Device {
        name: "NVIDIA GeForce GTX 1080 Ti".into(),
        memory_bytes: 11 * 1024 * 1024 * 1024,
        compute_capability: Some((6, 1)),
        free_bytes: None,
    }
}

/// `n` accelerators sharing one pool, with host memory alongside.
#[must_use]
pub fn discrete(devices: Vec<Device>, pooled: bool, host_gb: u64) -> Hardware {
    Hardware {
        topology: MemoryTopology::Discrete {
            devices,
            pooled,
            host_memory_bytes: host_gb * 1024 * 1024 * 1024,
        },
    }
}

/// A unified-memory host.
#[must_use]
pub fn unified(total_gb: u64) -> Hardware {
    Hardware {
        topology: MemoryTopology::Unified {
            total_bytes: total_gb * 1024 * 1024 * 1024,
        },
    }
}

/// The reference model: `qwen3.6-35b-a3b` at Q4, about 21 GB resident.
#[must_use]
pub fn reference_model() -> ModelRequirements {
    ModelRequirements {
        resident_bytes: 21 * 1024 * 1024 * 1024,
        requires_target_architecture: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The primary target ──────────────────────────────────────────────────

    /// The acceptance check, as a test: a 3090 owner gets the reference model.
    ///
    /// Note it needs the *headroom* to fit too — 21 GB of weights against 24 GB of
    /// card is 25.2 GB with the margin, which does NOT fit. That is the honest
    /// answer and the reason the margin exists: a model that fits its weights and
    /// not its KV cache OOMs partway through a long session, which is the worst
    /// time to find out.
    #[test]
    fn the_reference_model_needs_headroom_beyond_its_weights() {
        let one_3090 = discrete(vec![rtx_3090()], false, 64);
        let err = fits(&one_3090, &reference_model()).unwrap_err();
        assert!(
            matches!(err, Unfit::Memory { .. }),
            "21 GB of weights plus 20% headroom exceeds 24 GB; say so rather than OOMing later"
        );
        assert!(err.explain().contains("quantisation"));
    }

    /// A smaller quantisation of the same model fits one card comfortably.
    #[test]
    fn a_smaller_quantisation_fits_a_single_3090() {
        let one_3090 = discrete(vec![rtx_3090()], false, 64);
        let q3 = ModelRequirements {
            resident_bytes: 17 * 1024 * 1024 * 1024,
            requires_target_architecture: true,
        };
        fits(&one_3090, &q3).unwrap();
    }

    /// The near-term expansion. Two cards, pooled, hold the reference model easily.
    #[test]
    fn two_pooled_3090s_hold_the_reference_model() {
        let two = discrete(vec![rtx_3090(), rtx_3090()], true, 64);
        assert_eq!(two.accelerator_count(), 2);
        assert_eq!(two.usable_model_bytes(), 48 * 1024 * 1024 * 1024);
        fits(&two, &reference_model()).unwrap();
    }

    /// The distinction that makes `pooled` worth having: two cards that cannot share
    /// a model are not one big card.
    #[test]
    fn two_unpooled_cards_are_not_one_big_card() {
        let unpooled = discrete(vec![rtx_3090(), rtx_3090()], false, 64);
        assert_eq!(
            unpooled.usable_model_bytes(),
            24 * 1024 * 1024 * 1024,
            "without pooling the ceiling is the largest single device"
        );
        assert!(fits(&unpooled, &reference_model()).is_err());
    }

    // ── The stretch target ──────────────────────────────────────────────────

    /// A unified host has no separate VRAM to subtract from anything. Arithmetic
    /// that assumed two pools would refuse this machine.
    #[test]
    fn a_unified_memory_host_has_one_pool() {
        let host = unified(128);
        assert_eq!(host.usable_model_bytes(), 128 * 1024 * 1024 * 1024);
        fits(&host, &reference_model()).unwrap();

        let moe_120b = ModelRequirements {
            resident_bytes: 70 * 1024 * 1024 * 1024,
            requires_target_architecture: false,
        };
        fits(&host, &moe_120b).unwrap();
    }

    #[test]
    fn a_unified_host_is_not_refused_for_lacking_tensor_cores() {
        assert!(unified(128).meets_target_architecture());
    }

    // ── The degraded tier ───────────────────────────────────────────────────

    /// The dev box, through the ordinary path. No special case anywhere.
    #[test]
    fn the_dev_box_is_refused_for_the_reference_model_on_architecture() {
        let dev = discrete(vec![gtx_1080ti()], false, 64);
        let err = fits(&dev, &reference_model()).unwrap_err();
        assert!(
            matches!(err, Unfit::Architecture { .. }),
            "Pascal is older than the target architecture; that is the first reason it fails"
        );
        assert!(err.explain().contains("degraded tier"));
    }

    /// A model that does not require Ampere still fails the dev box on size alone
    /// once it is anywhere near the reference class.
    #[test]
    fn the_dev_box_is_also_too_small_for_the_reference_class() {
        let dev = discrete(vec![gtx_1080ti()], false, 64);
        let arch_agnostic = ModelRequirements {
            resident_bytes: 21 * 1024 * 1024 * 1024,
            requires_target_architecture: false,
        };
        assert!(matches!(
            fits(&dev, &arch_agnostic),
            Err(Unfit::Memory { .. })
        ));
    }

    /// And a 7B does run there — which is the whole point of a degraded tier rather
    /// than a hard refusal.
    #[test]
    fn a_small_model_does_run_on_the_dev_box() {
        let dev = discrete(vec![gtx_1080ti()], false, 64);
        let seven_b = ModelRequirements {
            resident_bytes: 5 * 1024 * 1024 * 1024,
            requires_target_architecture: false,
        };
        fits(&dev, &seven_b).unwrap();
    }

    #[test]
    fn no_accelerator_is_its_own_answer() {
        let none = discrete(vec![], false, 64);
        assert_eq!(fits(&none, &reference_model()), Err(Unfit::NoAccelerator));
    }

    // ── Admission: the rule that keeps the degraded tier honest ─────────────

    #[test]
    fn a_multi_step_task_is_admitted_on_a_multi_step_tier() {
        assert_eq!(
            admit(ToolCalling::MultiStep, TaskDemand::MultiStep, None),
            Admission::Admit
        );
    }

    /// The rule the user asked for explicitly: never silently downgrade.
    #[test]
    fn a_multi_step_task_on_a_single_shot_tier_escalates_rather_than_degrading() {
        let a = admit(
            ToolCalling::SingleShot,
            TaskDemand::MultiStep,
            Some("claude-opus-5"),
        );
        match a {
            Admission::Escalate { to, reason } => {
                assert_eq!(to, "claude-opus-5");
                assert!(reason.contains("multi_step") && reason.contains("single_shot"));
            }
            other => panic!("expected escalation, got {other:?}"),
        }
    }

    #[test]
    fn with_nowhere_to_escalate_it_refuses_plainly() {
        let a = admit(ToolCalling::SingleShot, TaskDemand::MultiStep, None);
        match a {
            Admission::Refuse { fix, .. } => assert!(fix.contains("reference model")),
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    /// A single-lookup task still runs on the degraded tier. Refusing everything
    /// would make the tier pointless.
    #[test]
    fn a_single_lookup_still_runs_on_a_single_shot_tier() {
        assert_eq!(
            admit(ToolCalling::SingleShot, TaskDemand::SingleLookup, None),
            Admission::Admit
        );
    }

    #[test]
    fn a_tier_with_no_tool_calling_runs_nothing() {
        assert!(matches!(
            admit(ToolCalling::None, TaskDemand::SingleLookup, None),
            Admission::Refuse { .. }
        ));
    }

    #[test]
    fn tool_calling_capability_is_ordered() {
        assert!(ToolCalling::None < ToolCalling::SingleShot);
        assert!(ToolCalling::SingleShot < ToolCalling::MultiStep);
    }

    // ── No dev-box assumptions leak into the core ───────────────────────────

    /// A guard against the failure the target correction exists to prevent: a
    /// constant tuned to 11 GB, or a single-device assumption, sitting in the
    /// resolver where it silently shapes everything downstream.
    #[test]
    fn the_resolver_holds_no_dev_box_constants() {
        let src = include_str!("hardware.rs");
        // Scan the module, not its tests: the guard's own assertion mentions the
        // constant it is looking for, and a test that trips on itself is noise.
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        let logic: String = src
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The 1080 Ti's 11 GB may appear only in its named fixture.
        let elevens = logic.matches("11 *").count();
        assert!(
            elevens <= 1,
            "an 11 GB constant appears {elevens} times in logic; the dev box must not \
             shape the core path"
        );
        assert!(
            !logic.contains("fp32") && !logic.contains("FP32"),
            "no FP32 fallback belongs in the core path"
        );
    }
}
