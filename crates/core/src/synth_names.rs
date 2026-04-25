//! Synthesized identifier names emitted into generated TypeScript.
//!
//! Every name the emit/analyze pipeline creates uses the `__svn_*`
//! prefix so it's trivially distinguishable from user code in
//! diagnostics. This module is the single source of truth for the
//! naming patterns; helpers here mean callers stay typo-safe and
//! the prefix can be retuned in one place if it ever needs to.
//!
//! Per-call-site uniqueness is by AST byte-offset (`<hex>` suffix),
//! which is stable across re-runs and collision-free as long as the
//! template parses unchanged.

/// Name of the synthetic template-check function the render body
/// wraps every template usage in. Registered once by `walk_template`,
/// consumed by emit and the JS-overlay branch's diagnostic muting.
pub const TPL_CHECK_FN: &str = "__svn_tpl_check";

/// `__svn_C_<hex>` — the locally-scoped component class returned by
/// `__svn_ensure_component(Comp)` at a `<Comp ...>` instantiation
/// site. Bytes-of-the-component-node-start act as the unique tag.
pub fn component_local(node_start: u32) -> String {
    format!("__svn_C_{node_start:x}")
}

/// `__svn_inst_<hex>` — the locally-scoped instance returned by
/// `new __svn_C_<hex>({...})`. Same tagging rule as
/// `component_local`. Only emitted when the instance has at least
/// one consumer (`$on(…)`, `bind:this=`, `let:foo` consumer).
pub fn instance_local(node_start: u32) -> String {
    format!("__svn_inst_{node_start:x}")
}
