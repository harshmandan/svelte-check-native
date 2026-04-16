//! Scoreboard — runs `cargo test` and prints a one-line parity summary.
//!
//! Phase 0.5 stub. Real implementation parses cargo test output, counts
//! passing vs. total across the upstream-sanity + bug-fixture suites, and
//! optionally patches `README.md` between the `<!-- SCOREBOARD-START -->` and
//! `<!-- SCOREBOARD-END -->` markers.

fn main() -> anyhow::Result<()> {
    eprintln!("scoreboard: not yet implemented (phase 0.5 stub)");
    eprintln!("placeholder count: 0/39 passing");
    Ok(())
}
