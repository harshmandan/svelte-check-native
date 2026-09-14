//! Runes-mode detection from a parsed script.
//!
//! Two consumers need the verdict and mirror two upstream rules:
//!
//! - svelte2tsx (`ExportedNames.isRunesMode`): the script references
//!   one of the `$state` / `$derived` / `$effect` globals, declares a
//!   top-level variable from `$props()`, or awaits outside any
//!   function. Every other rune (`$inspect`, `$host`, `$bindable`) is
//!   ignored.
//! - the Svelte compiler (`analyze/index.js`): any rune name is
//!   referenced, or an `await` sits outside a function.
//!
//! Both read identifier references, so a rune name inside a comment,
//! a string, a regex literal or a type reference never counts, and a
//! `$state` that resolves to a store subscription (`const state =
//! writable(…)`) is excluded through the caller's binding set.

use std::collections::HashSet;

use oxc_ast::ast::{
    ArrowFunctionExpression, AwaitExpression, CallExpression, Expression, Function,
    IdentifierReference, MethodDefinition, ObjectProperty, Program, PropertyKey, Statement,
    VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};

/// Which rule the probe applies — see the module doc.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunesRule {
    Svelte2tsx,
    Compiler,
}

const SVELTE2TSX_GLOBALS: &[&str] = &["$state", "$derived", "$effect"];
const COMPILER_RUNES: &[&str] = &[
    "$state",
    "$derived",
    "$props",
    "$bindable",
    "$effect",
    "$inspect",
    "$host",
];

/// Walks programs and expression parses looking for the runes signals
/// of one [`RunesRule`]. `found` is sticky across calls.
pub struct RunesProbe<'b> {
    rule: RunesRule,
    /// Top-level bindings of the component's scripts: a `$x` whose `x`
    /// is bound is a store subscription, not a rune.
    bound: &'b HashSet<String>,
    /// Nesting depth of function bodies; `await` only counts at 0.
    function_depth: u32,
    pub found: bool,
}

impl<'b> RunesProbe<'b> {
    pub fn new(rule: RunesRule, bound: &'b HashSet<String>) -> Self {
        Self {
            rule,
            bound,
            function_depth: 0,
            found: false,
        }
    }

    pub fn scan_program(&mut self, program: &Program<'_>) {
        if self.rule == RunesRule::Svelte2tsx {
            // Upstream `hasPropsRune`: only a top-level declaration
            // initialised by a `$props()` call (`handleVariableStatement`
            // looks at source-file children only).
            for stmt in &program.body {
                let decl = match stmt {
                    Statement::VariableDeclaration(d) => d,
                    Statement::ExportDeclaration(e) => match &e.declaration {
                        oxc_ast::ast::Declaration::VariableDeclaration(d) => d,
                        _ => continue,
                    },
                    _ => continue,
                };
                if declares_from_props_call(decl) {
                    self.found = true;
                }
            }
        }
        self.visit_program(program);
    }

    fn names(&self) -> &'static [&'static str] {
        match self.rule {
            RunesRule::Svelte2tsx => SVELTE2TSX_GLOBALS,
            RunesRule::Compiler => COMPILER_RUNES,
        }
    }

    /// A rune name is a global unless a binding of the base name makes
    /// it a store subscription (`const state = …` turns `$state` into
    /// one). Upstream removes globals by their store base name only, so
    /// a literal `function $state() {}` does not stop `$state` from
    /// counting.
    fn is_rune_global(&self, name: &str) -> bool {
        self.names().contains(&name) && !self.bound.contains(&name[1..])
    }

    fn check_key(&mut self, key: &PropertyKey<'_>) {
        // svelte2tsx's identifier walk also sees a method name
        // (`{ $state() {} }`) as a global; a plain property key is
        // skipped there.
        if self.rule == RunesRule::Svelte2tsx
            && let PropertyKey::StaticIdentifier(id) = key
            && self.is_rune_global(id.name.as_str())
        {
            self.found = true;
        }
    }
}

fn declares_from_props_call(decl: &VariableDeclaration<'_>) -> bool {
    decl.declarations.iter().any(|d| {
        matches!(
            &d.init,
            Some(Expression::CallExpression(call))
                if matches!(&call.callee, Expression::Identifier(id) if id.name == "$props")
        )
    })
}

impl<'a> Visit<'a> for RunesProbe<'_> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.is_rune_global(it.name.as_str()) {
            self.found = true;
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        walk::walk_call_expression(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if it.method {
            self.check_key(&it.key);
        }
        walk::walk_object_property(self, it);
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        self.check_key(&it.key);
        walk::walk_method_definition(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        if self.function_depth == 0 {
            self.found = true;
        }
        walk::walk_await_expression(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.function_depth += 1;
        walk::walk_function(self, it, flags);
        self.function_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, it);
        self.function_depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svn_parser::{ScriptLang, parse_script_body};

    fn probe(rule: RunesRule, src: &str) -> bool {
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        let mut bound = HashSet::new();
        crate::collect_top_level_bindings(&parsed.program, &mut bound);
        let mut p = RunesProbe::new(rule, &bound);
        p.scan_program(&parsed.program);
        p.found
    }

    #[test]
    fn svelte2tsx_rule() {
        let r = RunesRule::Svelte2tsx;
        assert!(probe(r, "let x = $state(0);"));
        assert!(probe(r, "let x = $derived.by(() => 1);"));
        assert!(probe(r, "let { a } = $props();"));
        assert!(probe(r, "export let { a } = $props();"));
        assert!(probe(r, "type T = typeof $state;"));
        assert!(probe(r, "const x = await f();"));
        assert!(probe(r, "const o = { $state() {} };"));
        assert!(!probe(r, "function f() { return $props(); }"));
        assert!(!probe(r, "$inspect(1);"));
        assert!(!probe(r, "// $state(0)\nlet x = 1;"));
        assert!(!probe(r, "const re = /$state(x)/;"));
        assert!(!probe(r, "const s = \"$state(0)\";"));
        assert!(!probe(r, "async function f() { await g(); }"));
        assert!(!probe(r, "const state = writable(0); $state.set(1);"));
        assert!(probe(r, "function $state(n) {} $state(1);"));
        assert!(!probe(r, "const o = { $state: 1 };"));
    }

    #[test]
    fn compiler_rule() {
        let r = RunesRule::Compiler;
        assert!(probe(r, "$inspect(1);"));
        assert!(probe(r, "function f() { return $props(); }"));
        assert!(probe(r, "$effect.pre(() => {});"));
        assert!(!probe(r, "const o = { $state() {} };"));
        assert!(!probe(r, "const state = writable(0); $state.set(1);"));
        assert!(!probe(r, "// $state(0)"));
    }
}
