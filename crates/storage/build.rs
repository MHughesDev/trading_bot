// sqlx::migrate! embeds migrations at compile time; new files must trigger a rebuild.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
