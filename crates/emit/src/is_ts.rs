//! Per-thread overlay-mode flag.
//!
//! Set at the top of `emit_document_with_render_name` via
//! [`IsTsGuard::enter`], read by deep emit sites (bind:this casts,
//! each-block `let i`, branch bindings, snippet params) via
//! [`emit_is_ts`]. The flag is per-thread because rayon parallelises
//! emission — each file runs on whatever rayon worker picks it up, so
//! the guard must set+reset per-invocation.
//!
//! Threading the flag through 9+ function signatures was the
//! alternative; the thread-local keeps every emit-helper signature
//! free of an `is_ts: bool` parameter that's only meaningful at one
//! conditional inside.

thread_local! {
    static EMIT_IS_TS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Snapshot the current value, replace it for the duration of the
/// guard's lifetime, and restore on drop. Constructed once per
/// `emit_document_with_render_name` call.
pub(crate) struct IsTsGuard {
    prev: bool,
}

impl IsTsGuard {
    pub(crate) fn enter(is_ts: bool) -> Self {
        let prev = EMIT_IS_TS.with(|c| c.replace(is_ts));
        Self { prev }
    }
}

impl Drop for IsTsGuard {
    fn drop(&mut self) {
        EMIT_IS_TS.with(|c| c.set(self.prev));
    }
}

/// Read the current overlay-mode flag. `true` for `.svelte.svn.ts`,
/// `false` for `.svelte.svn.js`. Default `true` if no guard is in
/// scope (test paths).
pub(crate) fn emit_is_ts() -> bool {
    EMIT_IS_TS.with(|c| c.get())
}
