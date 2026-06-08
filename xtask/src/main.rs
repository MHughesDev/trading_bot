//! Dev automation tasks.
//! Usage: cargo xtask <command>
//! Commands:
//!   check-money-f64  — scan workspace for f64 usage on price/size (CI enforced)

fn main() {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check-money-f64") => {
            println!("check-money-f64: stub — TODO(Phase B) implement f64 scanner");
        }
        Some(t) => {
            eprintln!("Unknown xtask: {t}");
            std::process::exit(1);
        }
        None => {
            println!("Usage: cargo xtask <check-money-f64>");
        }
    }
}
