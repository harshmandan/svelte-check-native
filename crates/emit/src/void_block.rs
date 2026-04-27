//! Void-block emission — the per-statement `void <name>;` lines
//! that keep TS6133 ("declared but never used") from firing on
//! every name our emit synthesizes (template-check wrapper, store
//! aliases, prop locals, etc.) AND on user names that ARE used,
//! just only from inside markup expressions.
//!
//! Two helpers live here:
//!
//! - [`emit_bind_pair_declarations`] — `__svn_bind_pair_N`
//!   getter/setter tuple placeholders, scoped INSIDE the
//!   `__svn_tpl_check` body so emit's `bind:foo={getter, setter}`
//!   destructure has something to read.
//! - [`emit_void_block`] — the outer-scope `void <name>;` set,
//!   covering the template-check wrapper, store auto-subscribe
//!   aliases, destructured props, and template-only refs.
//!
//! Both write into `&mut String` (`buf.raw_string_mut()` at the
//! call site) — they emit single-line statements with no overlay/
//! source-line bookkeeping needed.

use std::collections::HashSet;
use std::fmt::Write;

use smol_str::SmolStr;
use svn_analyze::TemplateSummary;

/// Emit getter/setter tuple placeholders for `bind:foo={getter,
/// setter}`. Same pattern as action-attrs: declare + void inside
/// `__svn_tpl_check`.
pub(crate) fn emit_bind_pair_declarations(
    out: &mut String,
    summary: &TemplateSummary,
    is_ts: bool,
) {
    for name in summary.void_refs.names() {
        if name.starts_with("__svn_bind_pair_") {
            if is_ts {
                let _ = writeln!(
                    out,
                    "        let {name}: [() => any, (v: any) => void] = [() => undefined as any, () => {{}}];"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        /** @type {{[() => any, (v: any) => void]}} */ let {name} = [() => undefined, () => {{}}];"
                );
            }
            let _ = writeln!(out, "        void {name};");
        }
    }
}

/// Emit the outer-scope void block.
///
/// One `void <name>;` statement per synthesized name — NOT a single
/// `void (a, b, c);` block. The block form uses comma operators which
/// TypeScript flags with TS2695 ("Left side of comma operator is
/// unused and has no side effects"). Per-statement form has no such
/// problem and matches what upstream svelte-check does.
///
/// Names covered:
///   - the template-check wrapper (`__svn_tpl_check`)
///   - store auto-subscribe aliases
///   - destructured props (the component's public API; treat as used
///     even if the body doesn't reference them directly)
///   - script-declared bindings that are referenced from the template
///     (component imports, locals only used in markup)
///
/// NOT covered here (intentionally): `__svn_action_attrs_N` and
/// `__svn_bind_pair_N`. Those names are declared *inside* the inner
/// `__svn_tpl_check` function and self-voided there. Voiding them
/// in the outer scope as well would fire TS2304 (cannot find name)
/// since the inner declarations aren't visible from the outer
/// function.
pub(crate) fn emit_void_block(
    out: &mut String,
    summary: &TemplateSummary,
    store_refs: &[SmolStr],
    prop_names: &[SmolStr],
    template_refs: &[SmolStr],
    exported_locals: &[SmolStr],
) {
    let mut emitted: HashSet<String> = HashSet::new();
    let mut emit = |out: &mut String, name: &str| {
        if emitted.insert(name.to_string()) {
            let _ = writeln!(out, "    void {name};");
        }
    };
    for name in summary.void_refs.names() {
        if name.starts_with("__svn_action_attrs_") || name.starts_with("__svn_bind_pair_") {
            continue;
        }
        emit(out, name);
    }
    for name in store_refs {
        emit(out, name);
        // The auto-subscribe alias `$store` references the store, but
        // the underlying `store` const is itself only used in template
        // expressions like `$store` (which the alias receives). Void
        // the base name so TS6133 doesn't fire on the original
        // declaration.
        if let Some(base) = name.strip_prefix('$') {
            emit(out, base);
        }
    }
    for name in prop_names {
        emit(out, name);
    }
    for name in template_refs {
        emit(out, name);
    }
    // Names declared `export const|let|var|function|class` (or
    // `export { x }`) by the user. Stripping the `export` keyword
    // leaves them as plain locals — without voiding, TS6133 fires
    // on the declaration. The user explicitly marked them as public
    // surface so counting them as "used" is the right call.
    for name in exported_locals {
        emit(out, name);
    }
}
