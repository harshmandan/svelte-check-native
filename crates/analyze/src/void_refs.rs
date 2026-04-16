//! Void-reference registry.
//!
//! Every name the emit crate synthesizes (`__svn_tpl_check`,
//! `__svn_action_attrs_N`, `__svn_bind_pair_N`, store aliases, prop locals,
//! template-referenced identifiers, etc.) is registered here during analyze.
//! Emit then writes a single consolidated `void (a, b, c, ...);` block at
//! the end of `$$render`, which is what stops `noUnusedLocals` from firing
//! on every synthesized name.
//!
//! Centralizing void-references avoids the trap where each new emit feature
//! has to remember to add a per-feature `void <name>;` line: the registry
//! collects names during analysis, emit reads it once, and adding a new
//! synthesized name is a single `.register()` call.
//!
//! Insertion is order-preserving and deduplicating. The order matters only
//! for stable test-snapshot comparisons; runtime semantics don't depend on it.

use smol_str::SmolStr;

/// Collector for names that need a `void <name>;` reference somewhere in
/// the generated TS.
#[derive(Debug, Clone, Default)]
pub struct VoidRefRegistry {
    /// Insertion-ordered, deduplicated list of names.
    names: Vec<SmolStr>,
}

impl VoidRefRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a name. Idempotent — duplicate inserts are no-ops.
    pub fn register(&mut self, name: impl Into<SmolStr>) {
        let name = name.into();
        if !self.names.iter().any(|n| n == &name) {
            self.names.push(name);
        }
    }

    /// All registered names in insertion order.
    pub fn names(&self) -> &[SmolStr] {
        &self.names
    }

    /// Number of registered names.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_iterate() {
        let mut r = VoidRefRegistry::new();
        r.register("a");
        r.register("b");
        r.register("c");
        assert_eq!(
            r.names(),
            &[SmolStr::from("a"), SmolStr::from("b"), SmolStr::from("c")]
        );
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn dedupes_on_register() {
        let mut r = VoidRefRegistry::new();
        r.register("a");
        r.register("a");
        r.register("b");
        r.register("a");
        assert_eq!(r.len(), 2);
        assert_eq!(r.names(), &[SmolStr::from("a"), SmolStr::from("b")]);
    }

    #[test]
    fn empty_registry() {
        let r = VoidRefRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.names().is_empty());
    }
}
