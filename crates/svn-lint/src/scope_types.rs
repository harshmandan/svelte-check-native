//! Public data types for the scope/binding model.
//!
//! Pulled out of `scope.rs` for navigability — these are pure value
//! types with no logic. The `ScopeTree` itself stays in `scope.rs`
//! because its private fields (`scopes`, `bindings`) are
//! manipulated directly by the `TreeBuilder` visitor that lives
//! there. `scope.rs` re-exports everything from this file via
//! `pub use crate::scope_types::*;`, so external callers continue to
//! reach types as `svn_lint::scope::Binding`, etc.

use smol_str::SmolStr;
use std::collections::HashMap;
use svn_core::Range;

/// Copy-id into the scope-tree arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

/// Copy-id into the binding arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BindingId(pub u32);

/// Mirrors upstream `BindingKind` (`types/index.d.ts:275`). Only the
/// variants we emit in walk-1 are represented; transform-only kinds
/// are left out until a rule needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    /// Plain `var` / `let` / `const` / `function` / `class` / `import`.
    Normal,
    /// `$props()` destructured field or bare identifier.
    Prop,
    /// `$bindable(default)` inside a `$props()` destructure.
    BindableProp,
    /// `$props()` rest element, OR `$$props` / `$$restProps` ambient.
    RestProp,
    /// `$state.raw(…)`.
    RawState,
    /// `$state(…)`.
    State,
    /// `$derived(…)` or `$derived.by(…)`.
    Derived,
    /// Store auto-subscribe binding (synthetic, from `$foo` references).
    StoreSub,
    /// `{#each items as item}` context or rest.
    Each,
    /// `{#snippet foo(x)}` parameter.
    Snippet,
    /// `{#await promise then value}` / `{@const X = …}` / `<Foo let:x>`.
    Template,
    /// `{#each items as item, i (i)}` index when keyed.
    Static,
    /// Non-runes `$: x = …` implicit declaration.
    LegacyReactive,
}

/// How the binding was declared. Mirrors upstream `DeclarationKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationKind {
    Var,
    Let,
    Const,
    Using,
    AwaitUsing,
    Function,
    Import,
    Param,
    RestParam,
    Synthetic,
}

/// Just enough info about the declarator's initializer that rules can
/// answer "is this a rune call? with what argument?" without re-walking
/// the AST.
#[derive(Clone, Debug)]
pub enum InitialKind {
    /// No initializer (`let x;`).
    None,
    /// Plain expression. The `primitive` flag is a conservative
    /// `should_proxy`-analog: `true` iff the expression is a Literal,
    /// TemplateLiteral, ArrowFunctionExpression, FunctionExpression,
    /// UnaryExpression, BinaryExpression, or `undefined`.
    Expression {
        primitive: bool,
    },
    /// `$state(x)` / `$state.raw(x)` / `$derived(x)` / `$props()` /
    /// `$bindable(x)` — rune call. `primitive_arg` is set for `$state`
    /// only (upstream's discriminator).
    RuneCall {
        rune: RuneCall,
        primitive_arg: bool,
    },
    /// `import …` — carries the source specifier and whether the
    /// binding is a default import (`import Foo from '...'`). Both
    /// are needed by `legacy_component_creation`, which fires only on
    /// default imports from `.svelte` files.
    Import {
        source: SmolStr,
        is_default: bool,
    },
    /// `function foo() {}`.
    FunctionDecl,
    /// `class Foo {}`.
    ClassDecl,
    /// each-block / snippet-block context (template-side).
    EachBlock,
    SnippetBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuneCall {
    State,
    StateRaw,
    Derived,
    DerivedBy,
    Props,
    Bindable,
    Inspect,
    Host,
    Effect,
}

/// An identifier reference resolved to a binding (via the parent-chain
/// walk in pass 2).
#[derive(Clone, Debug)]
pub struct Reference {
    /// Byte range of the reference identifier.
    pub range: Range,
    pub parent_kind: RefParentKind,
    /// Function depth at the reference site — matches
    /// `binding.scope.function_depth` iff ref is "same scope" for the
    /// `state_referenced_locally` check.
    pub function_depth_at_use: u32,
    /// True when the ref is the DIRECT callee argument of a `$state`
    /// or `$state.raw` rune call (nested deeper than one level —
    /// mirrors upstream's ancestor walk bug/behavior where a ref that
    /// IS the arg itself doesn't trigger the "derived" message).
    pub nested_in_state_call: bool,
    /// True when the ref is in a template attribute expression (i.e.
    /// not inside the `<script>` body proper).
    pub in_template: bool,
    /// True when the ref is inside an `{#if}`/`{#each}`/`{#await}`/
    /// `{#key}` — i.e. a control-flow block — as seen from the
    /// template walker.
    pub in_control_flow: bool,
    /// True when the ref is a `bind:this={name}` value identifier.
    pub is_bind_this: bool,
    /// True when the ref sits below a function closure boundary in the
    /// script AST (ArrowFn / FnExpr / FnDecl body). Needed for
    /// `non_reactive_update` which skips such refs.
    pub in_function_closure: bool,
    /// True when the identifier is the immediate callee of a
    /// `CallExpression` — `store_rune_conflict` uses this to fire
    /// only on rune-like call sites.
    pub parent_is_call: bool,
    /// True when the reference sits inside a top-level `$:` reactive
    /// statement in the instance script — `reactive_declaration_
    /// module_script_dependency` uses this.
    pub in_reactive_statement: bool,
    /// Ignore-stack snapshot at the reference's visit site — list of
    /// warning codes a `// svelte-ignore …` leading comment silenced
    /// for the enclosing statement. Rules consult this before
    /// emitting diagnostics. `None` == no ignores active (most refs).
    pub ignored: Option<Vec<SmolStr>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefParentKind {
    /// Plain read (e.g. `foo`, `fn(foo)`).
    Read,
    /// Object of a MemberExpression (`foo.x` / `foo[i]`). Matters for
    /// `state_referenced_locally` on rest-prop bindings: upstream
    /// fires on bare `restProp` references but not on `restProp.x`.
    MemberObject,
    /// `foo = …` — reassignment target.
    AssignmentLeft,
    /// `foo++` / `++foo` — reassignment target.
    UpdateTarget,
    /// `foo.x = …` — mutation (not reassignment).
    MemberObjectOfAssignment,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub scope: ScopeId,
    pub name: SmolStr,
    /// Range of the declaring identifier (not the whole declarator).
    pub range: Range,
    pub kind: BindingKind,
    pub declaration_kind: DeclarationKind,
    pub initial: InitialKind,
    pub references: Vec<Reference>,
    pub reassigned: bool,
    pub mutated: bool,
    /// Upstream `metadata.is_template_declaration`.
    pub is_template_declaration: bool,
    /// Upstream `metadata.inside_rest`: true when the binding sits
    /// inside a destructuring rest element (`[a, ...rest]`,
    /// `{...rest}`). Used by `bind_invalid_each_rest` to flag `bind:*`
    /// writes to rest-backed each-block bindings.
    pub inside_rest: bool,
    /// Post-walk alias for prop bindings like `export { klass as class }`
    /// in non-runes mode. Used by `export_let_unused` to filter out
    /// references that are just re-exports.
    pub prop_alias: Option<SmolStr>,
    /// True if any `bind:*={expr}` template directive's expression
    /// resolves to this binding as its base identifier. Drives
    /// `bind_invalid_each_rest`.
    pub has_bind_reference: bool,
    /// Pre-computed answer to "does `state_referenced_locally` fire on
    /// this binding under the user's svelte version?" Folds the
    /// upstream-version compat gate (`state_locally_fires_on_props`,
    /// `state_locally_rest_prop`) + the `State`-kind reassignment +
    /// primitive-initial check into one boolean. The rule then reads
    /// this field directly instead of re-deriving from `ctx.compat` at
    /// every call site.
    ///
    /// Populated at scope-build time via `build_with_template_and_runes`,
    /// after `reassigned` is finalised by the post-walk passes.
    /// Defaults to `false` in intermediate states; only consult after
    /// the scope tree is fully built.
    pub fires_state_referenced_locally: bool,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub function_depth: u32,
    pub declarations: HashMap<SmolStr, BindingId>,
}

impl Scope {
    pub(crate) fn new(parent: Option<ScopeId>, function_depth: u32) -> Self {
        Self {
            parent,
            function_depth,
            declarations: HashMap::new(),
        }
    }
}

/// An unresolved reference — an identifier use whose name didn't
/// match any declaration up the parent chain. Kept around for rules
/// that need to cross-check such references against other bindings
/// (e.g. `store_rune_conflict` reads these to find `$foo` → `foo`
/// conflicts).
#[derive(Clone, Debug)]
pub struct UnresolvedRef {
    pub name: SmolStr,
    pub range: Range,
    pub scope: ScopeId,
    pub parent_is_call: bool,
    /// Ignore-stack snapshot at the reference's visit site — same
    /// semantic as `Reference::ignored`. Store-sub synthesis copies
    /// this onto the synthetic binding's `references` entries so
    /// `store_rune_conflict` honours `// svelte-ignore` comments.
    pub ignored: Option<Vec<SmolStr>>,
}
