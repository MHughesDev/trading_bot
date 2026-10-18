//! Prints the `judgement` suite's scorecards as JSON (S-10.16).
//!
//! The caller is `evals/runner/run.py --suite judgement`. Scoring has to happen in
//! this process and not in Python because the answer key is generated from a seed
//! that never leaves the scorer (`eval::planted`), so a Python grader could only
//! be handed the answer or recompute it.
//!
//! * no flag: the cards a PR produces (open set only, compiled in);
//! * `--nightly [DIR]`: open and sealed cells together, read from `DIR`
//!   (default `evals/judgement/tapes`). Only the nightly job passes this.

use harness::eval::judgement_suite::{nightly_scorecards, pr_scorecards};

fn main() {
    let mut args = std::env::args().skip(1);
    let cards = match args.next().as_deref() {
        None => Ok(pr_scorecards()),
        Some("--nightly") => {
            let dir = args
                .next()
                .unwrap_or_else(|| "evals/judgement/tapes".to_string());
            nightly_scorecards(std::path::Path::new(&dir))
        }
        Some(other) => Err(format!("unknown argument `{other}`; use --nightly [DIR]")),
    };
    match cards {
        Ok(cards) => println!("{}", serde_json::to_string_pretty(&cards).expect("serialises")),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}
