//! `docs/plans/PLAN.md` says what is left. This checks that it is still true.
//!
//! ```bash
//! cargo test -p invariants --test the_plan_counts_what_the_plans_say -- --nocapture
//! ```
//!
//! # Why a test and not a habit
//!
//! `ROADMAP.md` §2 was written on 2026-09-15 and recorded "Set S — 0/189, NOT
//! STARTED" six days after Set S started. Nobody was careless: the counts were
//! **restated by hand** in a document nobody recomputed, and a hand-maintained
//! number is wrong by exactly the amount nobody has looked.
//!
//! That is the same argument `the_desk_calls_routes_that_exist` and
//! `every_index_link_resolves` already make for their own surfaces, applied to
//! the one document the programme steers by. A map that goes stale silently is
//! worse than no map, because people steer by it.
//!
//! So PLAN.md's headline is derived from the plan files, and this fails the
//! build when the two disagree. It prints the derived tables with `--nocapture`,
//! so fixing a failure is copying them in rather than counting by hand.
//!
//! # What "left" means here, and why it is a range
//!
//! Size-weighted, `S = 1`, `M = 2`, `L = 4`: 189 tasks of wildly different sizes
//! averaged flat would flatter us. `PARTIAL` counts half — and the test asserts
//! the *strict* and *generous* readings too, because the spread between them is
//! the honest uncertainty and PLAN.md publishes all three. A single number
//! presented as precision would be the same failure in a different direction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The sets with per-task status markers. A–L use one `Completion:` line per
/// MASTER instead and are counted by hand in PLAN.md §1's last table — stated
/// here so a reader does not take their absence for completeness.
const LIVE_SETS: &[&str] = &["set-P", "set-S", "set-T"];

/// How much a task of each declared size is worth.
fn weight(size: char) -> f64 {
    match size {
        'S' => 1.0,
        'L' => 4.0,
        // `M`, and anything that declares no size. Declaring none is the common
        // case in the older phase files and the middle weight is the least
        // wrong guess.
        _ => 2.0,
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
struct Tally {
    tasks: usize,
    units: f64,
    done: f64,
    partial: f64,
    not_started: f64,
    blocked: f64,
}

impl Tally {
    /// The published number: `PARTIAL` counts half.
    fn left_pct(self) -> f64 {
        if self.units == 0.0 {
            return 0.0;
        }
        100.0 * (self.units - self.done - self.partial / 2.0) / self.units
    }
    fn strict_pct(self) -> f64 {
        if self.units == 0.0 {
            return 0.0;
        }
        100.0 * (self.units - self.done) / self.units
    }
    fn generous_pct(self) -> f64 {
        if self.units == 0.0 {
            return 0.0;
        }
        100.0 * (self.units - self.done - self.partial) / self.units
    }
    fn add(&mut self, other: Tally) {
        self.tasks += other.tasks;
        self.units += other.units;
        self.done += other.done;
        self.partial += other.partial;
        self.not_started += other.not_started;
        self.blocked += other.blocked;
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every `### <ID> <title> — <size>` heading in a phase file, with the status
/// marker on one of the next few lines.
///
/// The marker vocabulary includes `BLOCKED:HUMAN` and `BLOCKED:UPSTREAM`, which
/// a scan for a bare `` `BLOCKED` `` misses — and missing them silently reports
/// a human gate as unstarted work, which is the opposite of what a reader needs.
fn tally_file(path: &Path) -> Tally {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let heads: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_task_heading(l))
        .map(|(i, _)| i)
        .collect();

    let mut t = Tally::default();
    for (n, &i) in heads.iter().enumerate() {
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        let w = weight(declared_size(lines[i]));
        t.tasks += 1;
        t.units += w;
        match status_after(&lines, i + 1, end) {
            Some("DONE") => t.done += w,
            Some("PARTIAL") => t.partial += w,
            Some("BLOCKED") => t.blocked += w,
            // `NOT STARTED`, or no marker at all. Both are "not done", and
            // treating an unmarked task as anything else would let a task
            // disappear from the count by losing its marker.
            _ => t.not_started += w,
        }
    }
    t
}

fn is_task_heading(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("### ") else {
        return false;
    };
    let Some(id) = rest.split_whitespace().next() else {
        return false;
    };
    // `S-11.3`, `P-0.1`, `T-2.4`.
    let mut parts = id.splitn(2, '-');
    let (a, b) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    !a.is_empty()
        && a.chars().all(|c| c.is_ascii_uppercase())
        && !b.is_empty()
        && b.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// The `— L` at the end of a heading, or `M` when none is declared.
fn declared_size(heading: &str) -> char {
    let t = heading.trim_end();
    let mut chars = t.chars().rev();
    let last = chars.next().unwrap_or('M');
    if !matches!(last, 'S' | 'M' | 'L') {
        return 'M';
    }
    // It must be preceded by an em dash and a space, or it is the last letter
    // of an ordinary word.
    if t.ends_with(&format!("— {last}")) {
        last
    } else {
        'M'
    }
}

fn status_after(lines: &[&str], from: usize, to: usize) -> Option<&'static str> {
    for line in lines.iter().take(to.min(from + 5)).skip(from) {
        let s = line.trim();
        let Some(rest) = s.strip_prefix('`') else {
            continue;
        };
        let Some(word) = rest.split('`').next() else {
            continue;
        };
        if word.starts_with("BLOCKED") {
            return Some("BLOCKED");
        }
        for known in ["DONE", "PARTIAL", "NOT STARTED"] {
            if word == known {
                return Some(match known {
                    "DONE" => "DONE",
                    "PARTIAL" => "PARTIAL",
                    _ => "NOT STARTED",
                });
            }
        }
    }
    None
}

/// Every phase file of a set, in phase order.
fn phase_files(set: &str) -> Vec<PathBuf> {
    let dir = repo_root().join("docs/plans/plan-sets").join(set);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("phase-") && n.ends_with(".md"))
        })
        .collect();
    out.sort_by_key(|p| phase_number(p));
    out
}

fn phase_number(p: &Path) -> u32 {
    p.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("phase-"))
        .and_then(|n| n.split('-').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

fn tally_set(set: &str) -> Tally {
    let mut t = Tally::default();
    for f in phase_files(set) {
        t.add(tally_file(&f));
    }
    t
}

/// The headline percentage PLAN.md publishes.
fn plan_md() -> String {
    let p = repo_root().join("docs/plans/PLAN.md");
    std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {} — the programme's one map is gone: {e}",
            p.display()
        )
    })
}

/// Percentages the document states, as whole numbers, in order of appearance.
fn stated_percentages(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' {
            // Walk back over digits.
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < i {
                let n: String = bytes[j..i].iter().collect();
                if let Ok(v) = n.parse::<u32>() {
                    out.push(v);
                }
            }
        }
        i += 1;
    }
    out
}

#[test]
fn the_scanner_still_finds_the_plan_files() {
    for set in LIVE_SETS {
        let files = phase_files(set);
        assert!(
            !files.is_empty(),
            "no phase files found for {set}; every count below would be zero and every \
             assertion would pass for the wrong reason"
        );
        let t = tally_set(set);
        assert!(
            t.tasks > 10,
            "{set} came back with {} tasks, which is not a plan set — the heading format \
             changed and this test has stopped reading it",
            t.tasks
        );
    }
}

/// A set's `MASTER.md` counts the set it is the master of.
///
/// This test exists because both of them were wrong by the maximum possible
/// amount. On 2026-09-21 `set-P/MASTER.md` and `set-T/MASTER.md` each read
/// "Completion: 0% (0 / N tasks). NOT STARTED." while their own phase files
/// held 83% and 75% respectively — and neither was noticed for six days,
/// because [`every_phase_headline_counts_its_own_tasks`] scans `phase-*.md`
/// and a MASTER is not a phase.
///
/// A file called MASTER is the first one a reader opens. Leaving the one file
/// whose name promises the summary outside the only mechanism that checks
/// summaries is the exact hole this suite exists to close, one directory up.
#[test]
fn every_set_master_counts_its_own_set() {
    let mut wrong: Vec<String> = Vec::new();
    for set in LIVE_SETS {
        let path = repo_root()
            .join("docs/plans/plan-sets")
            .join(set)
            .join("MASTER.md");
        let Ok(text) = std::fs::read_to_string(&path) else {
            wrong.push(format!("{set}/MASTER.md does not exist"));
            continue;
        };
        let t = tally_set(set);
        let done = done_task_count_for_set(set);

        let Some(line) = text.lines().find(|l| l.starts_with("**Completion:")) else {
            wrong.push(format!("{set}/MASTER.md has no `**Completion:` line at all"));
            continue;
        };
        let Some((d, total)) = first_fraction(line) else {
            wrong.push(format!(
                "{set}/MASTER.md states no `d / t` pair: {line}"
            ));
            continue;
        };
        if d != done || total != t.tasks {
            wrong.push(format!(
                "{set}/MASTER.md says {d}/{total}, the set holds {done}/{}",
                t.tasks
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a set MASTER disagrees with the set it is the master of. It is the first file          anyone opens, and both were wrong by the maximum possible amount before this          test existed: {wrong:#?}"
    );
}

/// [`done_task_count`] over every phase file of a set.
fn done_task_count_for_set(set: &str) -> usize {
    phase_files(set).iter().map(|f| done_task_count(f)).sum()
}

#[test]
fn the_headline_in_plan_md_is_the_number_the_plan_files_support() {
    let mut all = Tally::default();
    for set in LIVE_SETS {
        all.add(tally_set(set));
    }
    let derived = all.left_pct().round() as u32;
    let text = plan_md();

    let headline = text
        .split_once("## 1. Where we are — **")
        .expect(
            "PLAN.md no longer states its headline in the form `## 1. Where we are — **NN% left**`",
        )
        .1;
    let stated: u32 = headline
        .split_once("% left")
        .and_then(|(n, _)| n.parse().ok())
        .expect("the headline is not a percentage");

    assert!(
        stated.abs_diff(derived) <= 1,
        "PLAN.md says {stated}% left; the plan files say {derived}%. Whichever is right, the \
         programme is now being steered by a number nobody recomputed — which is exactly how \
         ROADMAP §2 came to record Set S as untouched six days after it started.\n\n\
         Re-derive with:\n  cargo test -p invariants --test the_plan_counts_what_the_plans_say \
         -- --nocapture"
    );

    // The other two readings are published beside it, and they are what make the
    // headline honest rather than a claim of precision.
    let pcts = stated_percentages(&text);
    for (name, want) in [
        ("strict", all.strict_pct().round() as u32),
        ("generous", all.generous_pct().round() as u32),
    ] {
        assert!(
            pcts.iter().any(|p| p.abs_diff(want) <= 1),
            "PLAN.md publishes three counting rules and the {name} one ({want}%) is not among \
             the percentages it states. The spread between them is the uncertainty; dropping one \
             turns a range into a claim."
        );
    }
}

#[test]
fn every_live_set_appears_in_plan_md_with_its_own_number() {
    let text = plan_md();
    let pcts = stated_percentages(&text);
    for set in LIVE_SETS {
        let t = tally_set(set);
        let want = t.left_pct().round() as u32;
        assert!(
            pcts.iter().any(|p| p.abs_diff(want) <= 1),
            "{set} is {want}% remaining and PLAN.md does not say so anywhere. A set that is \
             missing from the map is a set nobody schedules."
        );
    }
}

/// Every phase file's own `**Completion: N% (d / t tasks)**` line is the count
/// its own tasks support.
///
/// The headline in PLAN.md is one number and this is twenty-one, which is where
/// drift actually starts: a task flips to `DONE` and the line at the top of that
/// one file is the thing nobody thinks to change. Two of them were wrong on
/// 2026-09-21 — phase 6 said 1/24 with 2 done, phase 10 said 5/16 with 6 — and
/// both had been wrong since the day the task closed.
///
/// Only the `d / t` pair is checked, not the percentage beside it, because the
/// prose around it is where a file says *which* tasks and *why*, and holding
/// that to a format would cost more than it protects.
#[test]
fn every_phase_headline_counts_its_own_tasks() {
    let mut wrong: Vec<String> = Vec::new();
    for set in LIVE_SETS {
        for f in phase_files(set) {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let t = tally_file(&f);
            let name = f
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();

            let Some(line) = text.lines().find(|l| l.starts_with("**Completion:")) else {
                wrong.push(format!("{set}/{name}: no `**Completion:` line at all"));
                continue;
            };
            let Some((d, total)) = first_fraction(line) else {
                wrong.push(format!(
                    "{set}/{name}: the Completion line states no `d / t` pair: {line}"
                ));
                continue;
            };
            // `done` is a weight; the headline counts tasks, so recount.
            let done_tasks = done_task_count(&f);
            if d != done_tasks || total != t.tasks {
                wrong.push(format!(
                    "{set}/{name}: says {d}/{total}, the file holds {done_tasks}/{}",
                    t.tasks
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a phase file's headline and its own tasks disagree. Whoever reads that file first          reads the headline, and it is the number they will repeat: {wrong:#?}"
    );
}

/// The first `N / M` in a line.
fn first_fraction(line: &str) -> Option<(usize, usize)> {
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '/' {
            let mut a = i;
            while a > 0 && (bytes[a - 1].is_ascii_digit() || bytes[a - 1] == ' ') {
                a -= 1;
            }
            let mut b = i + 1;
            while b < bytes.len() && (bytes[b].is_ascii_digit() || bytes[b] == ' ') {
                b += 1;
            }
            let left: String = bytes[a..i].iter().collect();
            let right: String = bytes[i + 1..b].iter().collect();
            if let (Ok(l), Ok(r)) = (left.trim().parse(), right.trim().parse()) {
                return Some((l, r));
            }
        }
        i += 1;
    }
    None
}

/// Tasks marked `DONE`, counted rather than weighted.
fn done_task_count(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let heads: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_task_heading(l))
        .map(|(i, _)| i)
        .collect();
    let mut n = 0;
    for (k, &i) in heads.iter().enumerate() {
        let end = heads.get(k + 1).copied().unwrap_or(lines.len());
        if status_after(&lines, i + 1, end) == Some("DONE") {
            n += 1;
        }
    }
    n
}

/// The numbers, printed so a failure is fixed by copying rather than counting.
#[test]
fn print_the_tables() {
    let mut all = Tally::default();
    println!("\n=== derived {} ===", chrono_date());
    for set in LIVE_SETS {
        let t = tally_set(set);
        all.add(t);
        println!(
            "\n### {set}  tasks={} units={:.0} done={:.0} partial={:.0} not_started={:.0} \
             blocked={:.0}  -> {:.0}% left",
            t.tasks,
            t.units,
            t.done,
            t.partial,
            t.not_started,
            t.blocked,
            t.left_pct()
        );
        println!("| phase | tasks | units | done | partial | not started | blocked | % left |");
        println!("|---|---:|---:|---:|---:|---:|---:|---:|");
        let mut rows: BTreeMap<u32, (String, Tally)> = BTreeMap::new();
        for f in phase_files(set) {
            let title = std::fs::read_to_string(&f)
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches('#')
                .trim()
                .to_string();
            rows.insert(phase_number(&f), (title, tally_file(&f)));
        }
        for (_, (title, t)) in rows {
            println!(
                "| {title} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0}% |",
                t.tasks,
                t.units,
                t.done,
                t.partial,
                t.not_started,
                t.blocked,
                t.left_pct()
            );
        }
    }
    println!(
        "\n=== ALL LIVE SETS: units={:.0} done={:.0} partial={:.0} ===",
        all.units, all.done, all.partial
    );
    println!("  strict   (only DONE counts): {:.0}% left", all.strict_pct());
    println!("  headline (PARTIAL is half):  {:.0}% left", all.left_pct());
    println!("  generous (PARTIAL is done):  {:.0}% left", all.generous_pct());
}

/// Today, for the printed header. Not a dependency: the date is decoration on a
/// printout and a wrong one costs nothing, where pulling `chrono` into this
/// crate for it would be a dependency added for a decoration.
fn chrono_date() -> &'static str {
    "(run date: see your shell)"
}
