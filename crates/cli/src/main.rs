//! `svelte-check-native` — CLI entrypoint.
//!
//! Phase 0.5 stub. Prints a "not yet implemented" message to stderr and exits
//! with status `2` so the integration test harness can distinguish this from a
//! clean run (`0`) or a real diagnostic-found run (`1`).
//!
//! Real implementation lands in Phase 1+.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("svelte-check-native: not yet implemented (phase 0.5 stub)");
    eprintln!(
        "see todo.md in the repo root or https://github.com/harshmandan/svelte-check-native"
    );
    ExitCode::from(2)
}
