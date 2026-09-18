//! Scope / binding model for lint rules that need "what does this
//! identifier resolve to?" answers.
//!
//! Mirrors upstream `packages/svelte/src/compiler/phases/scope.js`'s
//! two-pass algorithm — declarations in walk-1, then a drain pass
//! resolves references + tags `reassigned`/`mutated`. See
//! `notes/lint.md §4.5` for the full design rationale.
//!
//! **Scope of this port (intentionally partial):**
//!
//! - Only the script scopes (module + instance) are modeled. Template
//!   scopes (`{#each}` / `{#snippet}` / `<Foo let:x>`) are not yet.
//! - `BindingKind::State` / `Derived` / `Prop` / `RestProp` /
//!   `BindableProp` / `RawState` folded into walk-1 rather than a
//!   separate `VariableDeclarator` pass.
//! - No constant-folding / `Evaluation`. The primitive/proxyable
//!   discriminator for `state_referenced_locally` uses a conservative
//!   static check (`should_proxy`-analog).
//! - No `blocker`, no `legacy_indirect_bindings`, no `prop_alias` —
//!   transform-only concepts.
//!
//! Enough for `component_name_lowercase`,
//! `attribute_global_event_reference`, `non_reactive_update`, and
//! `state_referenced_locally` to light up with upstream-byte parity.

use oxc_ast::ast::{
    ArrayPattern, AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression,
    ChainElement, Class, ClassBody, ClassElement, Expression, ForStatementInit, FunctionBody,
    IdentifierReference, LabeledStatement, ObjectExpression, ObjectPattern, ObjectPropertyKind,
    Program, PropertyKey, SimpleAssignmentTarget, Statement, UpdateExpression, VariableDeclaration,
    VariableDeclarator,
};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_core::Range;

use svn_parser::document::{Document, ScriptSection};
use svn_parser::parse_script_body;

use crate::codes::Code;
use crate::rules::script_ast_rules::{ErrorGate, ScriptRuleEvent, ScriptRuleHooks};
pub use crate::scope_rune_detection::is_rune_name;
use crate::scope_rune_detection::{
    detect_bindable_default, detect_rune_call_from_call, is_primitive_expr, is_primitive_rune_init,
    rune_keypath, state_rune_primitive_arg,
};
use crate::scope_util::{
    base_identifier, expression_from_default, expression_from_for_init,
    expression_from_property_key, idents_in_pattern, unwrap_ts_wrappers,
};

// Public data types live in `scope_types.rs`. Re-export them so
// external callers continue to reach `Binding`, `Scope`, etc. via
// the `crate::scope::` path. ScopeTree itself stays here because
// its private fields (`scopes`, `bindings`) are manipulated by the
// `TreeBuilder` visitor in this file — moving the struct definition
// would force those fields `pub(crate)`.
pub use crate::scope_types::*;

/// One scope tree per file; owns both script roots (module + instance).
pub struct ScopeTree {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    /// Root of the module script (if any), else equal to `instance_root`.
    pub module_root: ScopeId,
    /// Root of the instance script. Always present (may be an empty
    /// synthetic scope for module-only or template-only files).
    pub instance_root: ScopeId,
    /// Upstream `module.scope.references` — identifiers that never
    /// resolved to a declaration anywhere on the chain. Preserved so
    /// rules like `store_rune_conflict` can inspect them, and so the
    /// runes-mode resolver can look for surviving rune-named
    /// references (upstream `2-analyze/index.js:456`).
    pub unresolved_refs: Vec<UnresolvedRef>,
    /// True when an `await` occurs with no enclosing function in the
    /// INSTANCE script or a template expression — upstream's
    /// `has_await || instance.has_await` runes trigger (scope.js
    /// counts an AwaitExpression whose ancestor path has no
    /// Arrow/FunctionExpression/FunctionDeclaration; module-script
    /// awaits are NOT consulted).
    pub has_await: bool,
    /// Writes the compiler rejects (`validate_assignment`), in walk
    /// order: module script, instance script, then template. Template
    /// entries are drained by the template walk as it passes them.
    pub write_violations: Vec<WriteViolation>,
    /// `$props()` declarators that would fire
    /// `custom_element_props_identifier` when the file compiles as a
    /// custom element without an explicit `customElement.props`
    /// option. Upstream `VariableDeclarator.js:72-83` — the warning
    /// range is the id span (Identifier form) or the RestElement
    /// span (ObjectPattern-with-rest form). Stored here so the
    /// template-side walker can decide whether to fire once
    /// `custom_element_info` is known.
    pub custom_element_props_candidates: Vec<Range>,
    /// The ignore-stack snapshot at each candidate's site — mirrors
    /// `UnresolvedRef::ignored` / `Reference::ignored`.
    pub custom_element_props_ignored: Vec<Option<Vec<SmolStr>>>,
    /// Non-runes `export` facts harvested from the instance script's
    /// top level during its single parse (in `build_script_inner`),
    /// consumed by `promote_non_runes_exports` — which used to re-parse
    /// the whole instance script just to find these. `idents` are names
    /// from `export let/var …` (promoted to props unconditionally);
    /// `specs` are `export { local as alias }` pairs (promoted only when
    /// `local` resolves to a `Var`/`Let` binding, with `alias` applied
    /// when it differs from `local`).
    nonrunes_export_idents: Vec<SmolStr>,
    nonrunes_export_specs: Vec<(SmolStr, Option<SmolStr>)>,
    /// Local name of every `export { … }` specifier in the instance
    /// script, in either mode.
    export_spec_locals: Vec<SmolStr>,
    /// Script-AST rule events buffered by the [`ScriptRuleHooks`]
    /// callbacks during the module / instance script walks (see
    /// `crate::rules::script_ast_rules`). The walk itself emits no
    /// warnings — `walk_parsed` takes this buffer once the tree is
    /// final and flushes it at the stage where those rules emit.
    pub(crate) script_rule_events: Vec<crate::rules::script_ast_rules::ScriptRuleEvent>,
    /// The same rule events raised inside template expressions. The
    /// compiler's template walk runs the Literal / TemplateElement /
    /// NewExpression / … visitors on every `{…}` expression, so these
    /// surface in template order, interleaved with the element and
    /// block warnings. Sorted by source position.
    pub(crate) template_rule_events: Vec<crate::rules::script_ast_rules::ScriptRuleEvent>,
    /// The first error the compiler's scope builder raises while
    /// declaring bindings (`validate_identifier_name`): a `$` or
    /// `$`-prefixed name declared at the module / instance level.
    /// Scope creation precedes every analysis walk, so this error wins
    /// over anything the walks report.
    pub(crate) declaration_error: Option<(Code, String, Range)>,
    /// Every `$name` store-subscription reference (`$` and `$$name`
    /// excluded), as the compiler's store-subscription loop sees it
    /// before it declares the subscriptions.
    pub(crate) store_refs: Vec<StoreRef>,
    /// The source range of the `<script module>` body.
    pub(crate) module_script_range: Option<Range>,
    /// The instance script's top-level `$:` statements, in order.
    pub(crate) reactive_statements: Vec<ReactiveStatement>,
    /// Starts of the identifiers inside `$:` statements that are the
    /// target of a plain `=` assignment, directly or as the object of
    /// the assigned member (`a = …`, `a.b = …`) — references that do
    /// not make the statement depend on them.
    pub(crate) reactive_assignment_targets: std::collections::HashSet<u32>,
}

/// A top-level `$:` statement, as the compiler's legacy-mode ordering
/// of reactive statements sees it.
#[derive(Clone, Debug)]
pub(crate) struct ReactiveStatement {
    pub range: Range,
    /// The scope the statement opens.
    pub scope: ScopeId,
    /// The names it assigns (plain and destructured assignment
    /// targets, and the object of an updated member), each with the
    /// scope of the assignment, in order.
    pub assignments: Vec<(SmolStr, ScopeId)>,
}

/// One `$name` reference, with what `name` resolves to from it.
#[derive(Clone, Debug)]
pub(crate) struct StoreRef {
    pub name: SmolStr,
    pub range: Range,
    /// `name` (without the `$`) resolves from the reference to a
    /// declaration below the module and instance top levels — a
    /// subscription to it cannot be set up.
    pub nested_store: bool,
    pub parent_is_call: bool,
}

impl ScopeTree {
    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.0 as usize]
    }

    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.0 as usize]
    }

    pub fn all_bindings(&self) -> impl Iterator<Item = (BindingId, &Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .map(|(i, b)| (BindingId(i as u32), b))
    }

    /// Resolve `name` starting from `from`, walking the parent chain.
    pub fn resolve(&self, from: ScopeId, name: &str) -> Option<BindingId> {
        resolve_by_name(&self.scopes, from, name)
    }

    /// Like [`resolve`], but resolves against both the instance root
    /// and module root — used by template walkers that don't have a
    /// script-local scope to start from.
    pub fn resolve_from_template(&self, name: &str) -> Option<BindingId> {
        self.resolve(self.instance_root, name)
    }

    /// Innermost template scope whose recorded source range contains
    /// `offset`, falling back to `instance_root`. This is the lexical
    /// start point for resolving a reference at a given template
    /// position — mirrors upstream's `scope.get` walking from the node's
    /// own scope (svelte compiler shared/element.js). Script scopes have
    /// `range == None` and never match; template byte offsets are
    /// disjoint from script offsets anyway. The smallest-containing
    /// scope wins (innermost); between scopes with the same range the
    /// later-created one is nested inside the earlier, so it wins.
    pub fn innermost_template_scope_at(&self, offset: u32) -> ScopeId {
        let mut best = self.instance_root;
        let mut best_len = u32::MAX;
        for (i, s) in self.scopes.iter().enumerate() {
            if let Some(r) = s.range
                && r.start <= offset
                && offset < r.end
                && (r.end - r.start) <= best_len
            {
                best_len = r.end - r.start;
                best = ScopeId(i as u32);
            }
        }
        best
    }
}

/// Should the file be treated as runes mode for post-walk bookkeeping?
/// Caller controls this so it matches `ctx.runes` at the rules layer.
///
/// `compat` bakes the upstream-version gates into per-binding fields
/// at build time — see `Binding::fires_state_referenced_locally`. The
/// rule layer then reads that field directly instead of re-consulting
/// `compat` per binding.
#[allow(clippy::too_many_arguments)]
pub fn build_with_template_and_runes(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    runes: bool,
    compat: crate::compat::CompatFeatures,
    preprocess_ts: bool,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
    bidi_warned: Option<std::collections::HashSet<u32>>,
) -> ScopeTree {
    let mut tree = build_with_template(
        doc,
        fragment,
        source,
        runes,
        preprocess_ts,
        module_program,
        instance_program,
        bidi_warned,
    );
    if runes {
        // `ExportSpecifier.js`: in runes mode an exported local counts
        // as reassigned.
        for local in std::mem::take(&mut tree.export_spec_locals) {
            if let Some(bid) = tree.resolve(tree.instance_root, &local) {
                tree.bindings[bid.0 as usize].reassigned = true;
            }
        }
    } else {
        promote_non_runes_exports(&mut tree);
    }
    populate_compat_gated_fields(&mut tree, compat);
    tree
}

/// Set every binding's `fires_state_referenced_locally` flag based on
/// its kind + the user's svelte-version compat flags + (for `State`)
/// the reassignment / primitive-initial state. Must run AFTER all
/// post-walk passes that touch `reassigned` or kind — currently just
/// `promote_non_runes_exports` on the non-runes path.
fn populate_compat_gated_fields(tree: &mut ScopeTree, compat: crate::compat::CompatFeatures) {
    let primitive_by_ident: Vec<bool> = tree
        .bindings
        .iter()
        .map(|binding| match &binding.initial {
            InitialKind::RuneCall {
                primitive_arg: StateArg::Ident(name),
                ..
            } => ident_initial_is_primitive(tree, binding.scope, name),
            _ => false,
        })
        .collect();
    for (binding, by_ident) in tree.bindings.iter_mut().zip(primitive_by_ident) {
        binding.fires_state_referenced_locally = match binding.kind {
            BindingKind::RawState | BindingKind::Derived => true,
            BindingKind::Prop => compat.state_locally_fires_on_props,
            BindingKind::RestProp => {
                compat.state_locally_fires_on_props && compat.state_locally_rest_prop
            }
            BindingKind::State => {
                binding.reassigned || is_primitive_rune_init(&binding.initial) || by_ident
            }
            _ => false,
        };
    }
}

/// `should_proxy(identifier)`, negated: an identifier whose binding is
/// never reassigned and was initialised with a value the compiler does
/// not proxy is primitive; anything else (unresolved, reassigned, no
/// initialiser, a declaration or import) is proxied.
fn ident_initial_is_primitive(tree: &ScopeTree, scope: ScopeId, name: &str) -> bool {
    let Some(bid) = tree.resolve(scope, name) else {
        return false;
    };
    let b = tree.binding(bid);
    !b.reassigned
        && match &b.initial {
            InitialKind::Expression { primitive } => *primitive,
            _ => false,
        }
}

/// Promote Svelte-4 `export` declarations in the instance script to
/// `BindableProp` bindings. Consumes the export facts harvested during
/// the instance-script parse (`TreeBuilder::build_script_inner`) — no
/// re-parse. Runs only on the non-runes path.
fn promote_non_runes_exports(tree: &mut ScopeTree) {
    // `export let/var …` — promoted unconditionally (the `const`
    // exclusion was already applied at collection time).
    for name in std::mem::take(&mut tree.nonrunes_export_idents) {
        promote_to_bindable_prop(tree, tree.instance_root, &name);
    }
    // `export { local as alias }` — promote only when `local` resolves
    // to a `Var`/`Let` binding; apply the alias when present (it was
    // recorded only when it differs from the local name).
    for (local, alias) in std::mem::take(&mut tree.nonrunes_export_specs) {
        let Some(bid) = tree.resolve(tree.instance_root, &local) else {
            continue;
        };
        let is_var_let = matches!(
            tree.bindings[bid.0 as usize].declaration_kind,
            DeclarationKind::Var | DeclarationKind::Let
        );
        if is_var_let {
            promote_to_bindable_prop(tree, tree.instance_root, &local);
            if let Some(alias) = alias {
                tree.bindings[bid.0 as usize].prop_alias = Some(alias);
            }
        }
    }
}

fn promote_to_bindable_prop(tree: &mut ScopeTree, root: ScopeId, name: &str) {
    if let Some(bid) = tree.resolve(root, name) {
        let b = &mut tree.bindings[bid.0 as usize];
        b.kind = BindingKind::BindableProp;
    }
}

/// Find the ONE template comment immediately preceding a `<script>`
/// (whitespace-only text siblings allowed between) and return its
/// `svelte-ignore` codes. Mirrors upstream's parser
/// (`element.js:327-350`): it scans the fragment's nodes backward,
/// takes the FIRST comment it meets, and stops — of two stacked
/// comments only the nearest one bridges (verified against the
/// compiler), and the bridge applies to module and instance scripts
/// alike. Working off the parsed `Comment` nodes also keeps a body
/// containing a literal `<!--` intact, which a textual backward
/// scan for the opener would mis-pair.
pub(crate) fn collect_preceding_template_ignores(
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    script_start: u32,
    runes: bool,
) -> Vec<SmolStr> {
    use svn_parser::ast::Node;
    let Some(fragment) = fragment else {
        return Vec::new();
    };
    let mut end = script_start;
    for node in fragment.nodes.iter().rev() {
        let r = node.range();
        if r.end > end {
            // Node at/after the script tag.
            continue;
        }
        // The gap between this node and the cursor must be
        // whitespace-only (the runs around the extracted script
        // section aren't contiguous in our fragment).
        let Some(gap) = source.get(r.end as usize..end as usize) else {
            break;
        };
        if !gap.chars().all(char::is_whitespace) {
            break;
        }
        match node {
            Node::Comment(c) => {
                let body = c.data_range.slice(source);
                let trimmed = body.trim_start();
                let Some(rest) = trimmed.strip_prefix("svelte-ignore") else {
                    return Vec::new();
                };
                let rest = match rest.chars().next() {
                    Some(ch) if ch.is_whitespace() => &rest[ch.len_utf8()..],
                    _ => return Vec::new(),
                };
                return crate::ignore::parse_ignore_codes_public(rest, runes);
            }
            Node::Text(t) if t.range.slice(source).trim().is_empty() => {
                end = r.start;
            }
            _ => break,
        }
    }
    Vec::new()
}

/// Like [`build`], but also walks the template fragment — capturing
/// references in attribute expressions / interpolations / directive
/// values and the implicit reassignments from `bind:*` directives.
/// Callers that only need script-side information can use [`build`].
/// `module_program` / `instance_program` are the pre-parsed bodies of
/// the corresponding script sections — the caller (`walk_parsed`)
/// parses each section exactly once and shares the `Program` between
/// this builder and the script-AST rules. A `Some` script section is
/// always paired with a `Some` program.
#[allow(clippy::too_many_arguments)]
pub fn build_with_template(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    runes: bool,
    preprocess_ts: bool,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
    bidi_warned: Option<std::collections::HashSet<u32>>,
) -> ScopeTree {
    let mut tree_builder = TreeBuilder::new();
    tree_builder.runes = runes;
    tree_builder.preprocess_ts = preprocess_ts;
    tree_builder.bidi_warned = bidi_warned;

    // Module scope: if there's no module script at all we still create
    // a synthetic empty one so resolve() has a stable root. Matches
    // upstream's behavior — `create_scopes` always returns a scope
    // even for an empty Program body.
    let module_root = tree_builder.new_scope(None);
    if let Some(script) = &doc.module_script
        && let Some(program) = module_program
    {
        // A `<!-- svelte-ignore CODE -->` comment placed in the
        // template immediately before a `<script>` applies its codes
        // to the whole script body — module and instance alike.
        // Upstream wires this up in the parser (element.js sets the
        // Program's leadingComments); our sections parser extracts
        // scripts separately, so we bridge the ignore forward
        // explicitly.
        let leading = collect_preceding_template_ignores(
            fragment,
            doc.source,
            script.open_tag_range.start,
            runes,
        );
        tree_builder.build_script(script, program, module_root, runes, &leading);
    }

    let instance_root = tree_builder.new_scope(Some(module_root));
    if let Some(script) = &doc.instance_script
        && let Some(program) = instance_program
    {
        let leading = collect_preceding_template_ignores(
            fragment,
            doc.source,
            script.open_tag_range.start,
            runes,
        );
        tree_builder.build_script_as_instance(script, program, instance_root, runes, &leading);
    }

    if let Some(frag) = fragment {
        // Upstream template scope is a non-porous child of the
        // instance scope → function_depth = instance + 1. Mirror that
        // so template refs don't look like "same function_depth" as
        // instance-root bindings (important for
        // `state_referenced_locally`).
        // The walk opens the root fragment's own scope beneath this one
        // (stamped with the whole-fragment range), exactly as the
        // compiler's `Fragment` visitor does.
        let template_root = tree_builder.new_scope(Some(instance_root));
        let lang = doc
            .instance_script
            .as_ref()
            .map(|s| s.lang)
            .unwrap_or(svn_parser::document::ScriptLang::Js);
        tree_builder.walk_template(frag, source, template_root, lang);
    }

    let mut tree = tree_builder.finish(module_root, instance_root);
    tree.module_script_range = doc.module_script.as_ref().map(|s| s.content_range);
    tree
}

struct TreeBuilder {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    /// Pass-2 work queue: (scope, name, offset-within-script, base_offset,
    /// parent_kind, function_depth_at_use, nested_in_state, in_fn_closure).
    pending_refs: Vec<PendingRef>,
    pending_updates: Vec<PendingUpdate>,
    pending_writes: Vec<PendingWrite>,
    /// Identifiers assigned by a top-level `$: x = …` in the instance
    /// script. Upstream collects them as `possible_implicit_declarations`
    /// and, once the script walk is done, declares each one without an
    /// outer binding as `legacy_reactive`.
    implicit_reactive_decls: Vec<(SmolStr, Range)>,
    /// See [`ScopeTree::reactive_statements`].
    reactive_statements: Vec<ReactiveStatement>,
    /// See [`ScopeTree::reactive_assignment_targets`].
    reactive_assignment_targets: Vec<u32>,
    /// Accumulated `$props()` identifier / rest-element ranges that
    /// would fire `custom_element_props_identifier` when the file
    /// compiles as a custom element. Paired with an ignore-stack
    /// snapshot so `// svelte-ignore` leading comments are honoured.
    custom_element_props_candidates: Vec<Range>,
    custom_element_props_ignored: Vec<Option<Vec<SmolStr>>>,
    /// Non-runes instance-script `export` facts, collected during the
    /// instance parse (see the matching `ScopeTree` fields).
    nonrunes_export_idents: Vec<SmolStr>,
    nonrunes_export_specs: Vec<(SmolStr, Option<SmolStr>)>,
    /// Local name of every `export { … }` specifier in the instance
    /// script, in either mode.
    export_spec_locals: Vec<SmolStr>,
    /// Arena reused across the per-expression template mini-parses in
    /// `walk_expr_range`. Templates carry hundreds of tiny `{expr}`
    /// slices per file; constructing a fresh oxc Allocator for each
    /// costs more than the parse itself, so one arena is taken out of
    /// this slot per parse, `reset()` (which keeps its largest chunk),
    /// and put back. `None` only while a parse is in flight.
    expr_alloc: Option<oxc_allocator::Allocator>,
    /// The next template expression walked is the initializer of a
    /// declaration tag (`{let x = …}` / `{const x = …}`), a variable
    /// declarator the way `{@const}` is not: a rune call may stand
    /// there.
    declaration_tag_init: bool,
    /// Bindings our walk declares on entering a scope that the compiler
    /// declares only after walking the scope's contents (the
    /// `{:then}` / `{:catch}` values).
    late_declared: Vec<BindingId>,
    /// See [`ScopeTree::has_await`].
    has_await: bool,
    /// See [`ScopeTree::template_rule_events`].
    template_rule_events: Vec<ScriptRuleEvent>,
    /// See [`ScopeTree::declaration_error`].
    declaration_error: Option<(Code, String, Range)>,
    /// The literals the compiler's stateful bidi search reports (see
    /// `bidi_state`); `None` when the file holds no bidi character.
    bidi_warned: Option<std::collections::HashSet<u32>>,
    /// The project's preprocessors transpile `<script lang="ts">`
    /// (see `typescript_features::script_is_transpiled`).
    preprocess_ts: bool,
    /// The file's runes mode, for the template-expression rule hooks.
    runes: bool,
    /// See [`ScopeTree::script_rule_events`] — filled by the
    /// [`ScriptRuleHooks`] callbacks during the script walks.
    script_rule_events: Vec<ScriptRuleEvent>,
}

struct PendingRef {
    scope: ScopeId,
    name: SmolStr,
    range: Range,
    parent_kind: RefParentKind,
    function_depth_at_use: u32,
    nested_in_state_call: bool,
    in_function_closure: bool,
    in_template: bool,
    in_control_flow: bool,
    is_bind_this: bool,
    parent_is_call: bool,
    in_reactive_statement: bool,
    /// Snapshot of the ignore stack at the time this reference was
    /// recorded. `None` when no ignores were active (cheap for the
    /// common case). Mirrors upstream's `ignore_map` per-node
    /// snapshot — walkers push leading-comment `svelte-ignore` codes
    /// when entering a statement and pop on exit.
    ignored: Option<Vec<SmolStr>>,
}

#[derive(Clone, Copy, Default)]
struct RefFlags {
    /// `bind:this={name}` — value is the backing ident of a bind-this.
    is_bind_this: bool,
}

#[derive(Clone, Copy)]
struct TemplateCtx<'src> {
    source: &'src str,
    scope: ScopeId,
    lang: svn_parser::document::ScriptLang,
    /// True when the current sub-fragment sits beneath an
    /// `{#if}` / `{#each}` / `{#await}` / `{#key}` block — tracked so
    /// `non_reactive_update`'s bind:this subcase can tell when a
    /// write affects reactive dependencies.
    in_control_flow: bool,
}

/// Which walk a write was found in. The compiler walks the module
/// script, the instance script and then the template, and stops at the
/// first error, so this orders the write errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOrigin {
    Module,
    Instance,
    Template,
}

/// A write the compiler's `validate_assignment` rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteViolationKind {
    /// `constant_assignment` / `constant_binding`: the target is an
    /// import, or a `const` that is not an each-block item.
    Constant { import: bool },
    /// `each_item_invalid_assignment` — runes mode only.
    EachItem,
    /// `snippet_parameter_assignment`.
    SnippetParameter,
}

#[derive(Clone, Debug)]
pub struct WriteViolation {
    pub kind: WriteViolationKind,
    /// The whole assignment, update or `bind:` directive.
    pub range: Range,
    pub is_binding: bool,
    pub origin: WriteOrigin,
}

/// An assignment, update or `bind:` whose identifier targets still
/// need resolving before the compiler's write checks can run.
struct PendingWrite {
    scope: ScopeId,
    /// Identifiers the compiler checks, in its order: a bare target, or
    /// the plain identifiers of a destructuring target (defaults and
    /// rest elements are not checked).
    targets: Vec<SmolStr>,
    /// The target is a single identifier, not a pattern.
    bare: bool,
    range: Range,
    is_binding: bool,
    origin: WriteOrigin,
}

struct PendingUpdate {
    scope: ScopeId,
    /// Name of the base identifier being written. For `foo = …` that's
    /// `foo`; for `foo.bar = …` that's `foo` (mutation, not reassign);
    /// for `foo.bar.baz = …` also `foo` (mutation).
    name: SmolStr,
    range: Range,
    /// True for `foo = …` / `foo++` — reassignment. False for
    /// `foo.x = …` — mutation only.
    is_reassign: bool,
}

impl TreeBuilder {
    fn new() -> Self {
        Self {
            scopes: Vec::new(),
            bindings: Vec::new(),
            pending_refs: Vec::new(),
            pending_updates: Vec::new(),
            pending_writes: Vec::new(),
            implicit_reactive_decls: Vec::new(),
            reactive_statements: Vec::new(),
            reactive_assignment_targets: Vec::new(),
            custom_element_props_candidates: Vec::new(),
            custom_element_props_ignored: Vec::new(),
            nonrunes_export_idents: Vec::new(),
            nonrunes_export_specs: Vec::new(),
            export_spec_locals: Vec::new(),
            expr_alloc: None,
            declaration_tag_init: false,
            late_declared: Vec::new(),
            has_await: false,
            script_rule_events: Vec::new(),
            template_rule_events: Vec::new(),
            declaration_error: None,
            runes: false,
            preprocess_ts: false,
            bidi_warned: None,
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let depth = match parent {
            Some(pid) => self.scopes[pid.0 as usize].function_depth + 1,
            None => 0,
        };
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope::new(parent, depth, false));
        id
    }

    fn new_porous_scope(&mut self, parent: ScopeId) -> ScopeId {
        let depth = self.scopes[parent.0 as usize].function_depth;
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope::new(Some(parent), depth, true));
        id
    }

    fn declare(
        &mut self,
        scope: ScopeId,
        name: SmolStr,
        range: Range,
        kind: BindingKind,
        declaration_kind: DeclarationKind,
        initial: InitialKind,
    ) -> BindingId {
        // `var` hoists through porous (block-like) scopes to the
        // nearest function/root scope — upstream `scope.js` forwards
        // the declaration to the parent while the scope is porous.
        let mut scope = scope;
        if declaration_kind == DeclarationKind::Var {
            while self.scopes[scope.0 as usize].porous {
                let Some(parent) = self.scopes[scope.0 as usize].parent else {
                    break;
                };
                scope = parent;
            }
        }
        // A name declared twice in one scope, neither time with `var`,
        // is a compile error (the scope builder's own check; plain
        // JavaScript duplicates fail to parse before this).
        if self.declaration_error.is_none()
            && declaration_kind != DeclarationKind::Var
            && let Some(existing) = self.scopes[scope.0 as usize].declarations.get(&name)
            && self.bindings[existing.0 as usize].declaration_kind != DeclarationKind::Var
        {
            // The compiler declares an `{:then}` / `{:catch}` value
            // only after walking the branch, so a clash with a
            // declaration inside the branch points at the value.
            let at = if self.late_declared.contains(existing) {
                self.bindings[existing.0 as usize].range
            } else {
                range
            };
            self.declaration_error = Some((
                Code::declaration_duplicate,
                crate::messages::declaration_duplicate(&name),
                at,
            ));
        }
        // `validate_identifier_name(binding, scope.function_depth)`:
        // outside parameters and synthetic bindings, a name at the
        // module or instance level (function depth <= 1, which block
        // scopes do not raise) may not be `$` or start with `$`.
        if self.declaration_error.is_none()
            && self.scopes[scope.0 as usize].function_depth <= 1
            && !matches!(
                declaration_kind,
                DeclarationKind::Synthetic | DeclarationKind::Param | DeclarationKind::RestParam
            )
            && let Some(error) = dollar_name_error(&name)
        {
            self.declaration_error = Some((error.0, error.1, range));
        }
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            scope,
            name: name.clone(),
            range,
            kind,
            declaration_kind,
            initial,
            references: Vec::new(),
            reassigned: false,
            mutated: false,
            is_template_declaration: false,
            inside_rest: false,
            prop_alias: None,
            bind_reference_count: 0,
            fires_state_referenced_locally: false,
            ignored: None,
        });
        self.scopes[scope.0 as usize].declarations.insert(name, id);
        id
    }

    /// Walk the template fragment, extracting every expression-bearing
    /// site and feeding it through a lightweight script-body-like
    /// walker. Reference flags (`in_template`, `in_control_flow`,
    /// `is_bind_this`) are threaded through so
    /// `non_reactive_update` can decide which references to trust.
    ///
    /// Drives an internal [`LintScopeVisitor`] over the unified
    /// [`svn_analyze::template_scope::walk_with_visitor`] walker.
    /// Per-block scope creation, binding declarations, and
    /// expression walking happen inside the visitor's `visit_*`
    /// methods; the walker handles structural recursion and
    /// scope/control-flow bracketing.
    fn walk_template(
        &mut self,
        fragment: &svn_parser::ast::Fragment,
        source: &str,
        instance_root: ScopeId,
        lang: svn_parser::document::ScriptLang,
    ) {
        let mut visitor = LintScopeVisitor {
            builder: self,
            ctx: TemplateCtx {
                source,
                scope: instance_root,
                lang,
                in_control_flow: false,
            },
            scope_stack: Vec::new(),
            control_flow_stack: Vec::new(),
        };
        svn_analyze::template_scope::walk_with_visitor(fragment, source, &mut visitor);
    }

    fn walk_template_attr(&mut self, attr: &svn_parser::ast::Attribute, ctx: &mut TemplateCtx<'_>) {
        use svn_parser::ast::{AttrValuePart, Attribute, DirectiveKind};
        match attr {
            Attribute::Plain(p) => {
                if let Some(v) = &p.value {
                    for part in &v.parts {
                        if let AttrValuePart::Expression {
                            expression_range, ..
                        } = part
                        {
                            self.walk_expr_range(*expression_range, ctx, RefFlags::default());
                        }
                    }
                }
            }
            Attribute::Expression(e) => {
                self.walk_expr_range(e.expression_range, ctx, RefFlags::default());
            }
            Attribute::Shorthand(s) => {
                // `{name}` — single identifier ref at template root.
                self.record_template_ref(s.name.as_str(), s.range, ctx, RefFlags::default());
            }
            Attribute::Spread(s) => {
                self.walk_expr_range(s.expression_range, ctx, RefFlags::default());
            }
            Attribute::Comment(_) => {}
            Attribute::Directive(d) => {
                let flags = RefFlags {
                    is_bind_this: d.kind == DirectiveKind::Bind && d.name == "this",
                };
                // Directives whose NAME is implicitly an identifier
                // reference: `use:action`, `transition:fn`, `in:fn`,
                // `out:fn`, `animate:fn`. The name is the function
                // the user imports/declares; the directive passes it
                // to Svelte. Without recording this, a top-level
                // `let fn = …` used only as `use:fn` looks unused
                // and fires `export_let_unused` / similar.
                if matches!(
                    d.kind,
                    DirectiveKind::Use
                        | DirectiveKind::Transition
                        | DirectiveKind::In
                        | DirectiveKind::Out
                        | DirectiveKind::Animate
                ) {
                    // `use:tooltips.show` references `tooltips`.
                    let root = d.name.split('.').next().unwrap_or_default();
                    self.record_template_ref(root, d.range, ctx, RefFlags::default());
                }
                match &d.value {
                    Some(svn_parser::ast::DirectiveValue::Expression {
                        expression_range, ..
                    }) => {
                        self.walk_expr_range(*expression_range, ctx, flags);
                        if d.kind == DirectiveKind::Bind {
                            self.register_bind_update(*expression_range, d.range, ctx);
                        }
                    }
                    Some(svn_parser::ast::DirectiveValue::BindPair {
                        getter_range,
                        setter_range,
                        ..
                    }) => {
                        self.walk_expr_range(*getter_range, ctx, flags);
                        self.walk_expr_range(*setter_range, ctx, flags);
                    }
                    Some(svn_parser::ast::DirectiveValue::Quoted(v)) => {
                        for part in &v.parts {
                            if let AttrValuePart::Expression {
                                expression_range, ..
                            } = part
                            {
                                self.walk_expr_range(*expression_range, ctx, flags);
                            }
                        }
                        // The parser reads `bind:x="{y}"` as `bind:x={y}`.
                        if d.kind == DirectiveKind::Bind
                            && let [
                                AttrValuePart::Expression {
                                    expression_range, ..
                                },
                            ] = v.parts.as_slice()
                        {
                            self.register_bind_update(*expression_range, d.range, ctx);
                        }
                    }
                    None => {
                        match d.kind {
                            DirectiveKind::Bind => {
                                // `bind:foo` shorthand — implicit
                                // `{foo}` identifier reference +
                                // reassignment.
                                self.record_template_ref(d.name.as_str(), d.range, ctx, flags);
                                self.pending_updates.push(PendingUpdate {
                                    scope: ctx.scope,
                                    name: SmolStr::from(d.name.as_str()),
                                    range: d.range,
                                    is_reassign: true,
                                });
                                self.pending_writes.push(PendingWrite {
                                    scope: ctx.scope,
                                    targets: vec![SmolStr::from(d.name.as_str())],
                                    bare: true,
                                    range: d.range,
                                    is_binding: true,
                                    origin: WriteOrigin::Template,
                                });
                            }
                            // `class:foo` / `style:foo` without value
                            // are shorthand for `class:foo={foo}` /
                            // `style:foo={foo}` — an implicit read of
                            // the identifier in the current scope.
                            // Without recording this, props used only
                            // via class/style directives look unused
                            // to `export_let_unused`.
                            DirectiveKind::Class | DirectiveKind::Style => {
                                self.record_template_ref(d.name.as_str(), d.range, ctx, flags);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Walk the body of a `{@const NAME = EXPR}` tag. Re-parses the
    /// body as `let NAME = EXPR;` so the declared name lands in the
    /// current scope as a Template-kind binding (upstream `scope.js`
    /// declares these with `kind: 'template'`) — NOT as a write to
    /// the outer `NAME` binding.
    fn walk_const_tag(&mut self, range: Range, ctx: &mut TemplateCtx<'_>) {
        let Some(slice) = ctx.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let wrapped = format!("let {slice};");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, &wrapped, ctx.lang);
        let offset: i32 = range.start as i32 - 4;
        if let Some(Statement::VariableDeclaration(vd)) = parsed.program.body.first()
            && let Some(d) = vd.declarations.first()
        {
            // Declare every pattern identifier in the current
            // template scope, without feeding them through
            // visit_assignment. Capture the binding-id boundary
            // BEFORE the declaration so the retag below only touches
            // the names this `{@const}` introduced — without that,
            // existing each-block bindings already in scope would be
            // retagged from Each to Template, suppressing every
            // each-specific lint check after the first `{@const}`.
            let new_id_start = self.bindings.len() as u32;
            self.declare_each_pattern(
                &d.id, ctx.scope, ctx.scope, offset, false, ctx.source, ctx.lang,
            );
            let new_id_end = self.bindings.len() as u32;
            // Re-tag JUST the bindings declare_each_pattern just
            // appended (it uses `BindingKind::Each` internally as a
            // generic pattern marker; for `{@const}` the right kind
            // is Template).
            for idx in new_id_start..new_id_end {
                if matches!(self.bindings[idx as usize].kind, BindingKind::Each) {
                    self.bindings[idx as usize].kind = BindingKind::Template;
                }
            }
            // Walk the initializer expression so refs inside
            // resolve to the outer scope.
            if let Some(init) = &d.init {
                let init_span = {
                    use oxc_span::GetSpan;
                    init.span()
                };
                let abs = Range::new(
                    (init_span.start as i32 + offset).max(0) as u32,
                    (init_span.end as i32 + offset).max(0) as u32,
                );
                // `{@const` keeps its `@`; a declaration tag has only
                // the keyword before the pattern.
                let tag_head = ctx.source[..range.start as usize]
                    .rsplit('{')
                    .next()
                    .unwrap_or_default();
                self.declaration_tag_init = !tag_head.trim_start().starts_with('@');
                self.walk_expr_range(abs, ctx, RefFlags::default());
                self.declaration_tag_init = false;
            }
        }
        drop(parsed);
        drop(alloc);
    }

    fn walk_expr_range(&mut self, range: Range, ctx: &mut TemplateCtx<'_>, flags: RefFlags) {
        let Some(slice) = ctx.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        // Fast path: the dominant template-expression shape is a bare
        // identifier (`{name}`, `bind:value={name}`, …). Parsing one
        // with oxc yields exactly one Read-kind `PendingRef` with no
        // closure/state/ignore flags — the same record the Shorthand
        // fast path in `walk_template_attr` makes — so record it
        // directly and skip the parser. Reserved words fall through
        // to the parser: oxc treats them as literals (`true`, `null`,
        // `this`) or rejects them (`let`, `await`, …), so they must
        // not be recorded as identifier references.
        if let Some((tok_start, token)) = bare_identifier(slice)
            && !is_reserved_word(token)
            && token != "arguments"
            && !is_rune_name(token)
        {
            let abs_start = range.start + tok_start as u32;
            let tok_range = Range::new(abs_start, abs_start + token.len() as u32);
            self.record_template_ref(token, tok_range, ctx, flags);
            return;
        }
        // Template expression slices that start with `{` are object
        // literals in Svelte's grammar (`use:foo={{ a: b }}`,
        // `style={{ x: y }}`), but at program-body level oxc parses
        // `{` as a BlockStatement and then fails on `a: b, c: d`
        // (labelled-statement + comma is a parse error). Wrap those
        // in parens to force expression parsing. Adjust `base_offset`
        // backward by the wrapping-prefix length so that the absolute
        // positions we record for identifiers remain the source's
        // original offsets.
        let leading = slice
            .bytes()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
        // A slice starting with a string literal needs the same
        // treatment: at statement level oxc reads it as a directive
        // prologue (`"use strict"`), not an expression, and the
        // literal would never be visited.
        let needs_wrap = matches!(
            leading.and_then(|i| slice.as_bytes().get(i).copied()),
            Some(b'{' | b'"' | b'\'')
        );
        let wrapped: String;
        let (effective_slice, base_adjust): (&str, u32) = if needs_wrap && range.start > 0 {
            wrapped = format!("({slice})");
            (wrapped.as_str(), 1)
        } else {
            (slice, 0)
        };
        // Reuse one arena across all of this builder's expression
        // parses (see the `expr_alloc` field doc).
        let mut alloc = self.expr_alloc.take().unwrap_or_default();
        let parsed = parse_script_body(&alloc, effective_slice, ctx.lang);
        let start_depth = self.scopes[ctx.scope.0 as usize].function_depth;
        let runes = self.runes;
        let mut walker = ScriptWalker {
            tree: self,
            // The prepended `(` shifts every oxc span by +1; offset
            // `base_offset` by -1 so `base_offset + span.start`
            // still lands at the correct source byte.
            base_offset: range.start - base_adjust,
            scope_stack: vec![ctx.scope],
            function_depth: start_depth,
            rune_bump: 0,
            in_function_closure: false,
            in_state_arg_nested: false,
            in_reactive_statement: false,
            is_instance: false,
            at_program_top: false,
            // Template expression slices rarely carry
            // `// svelte-ignore` comments (they're inside `{…}`),
            // so skip the precollect for perf.
            script_comments: crate::ignore::ScriptComments::empty(),
            script_content: effective_slice,
            ignore_frames: Vec::new(),
            // Template expressions count toward the runes await
            // trigger (upstream: fragment create_scopes has_await).
            counts_await: true,
            // The compiler's template walk runs the same expression
            // visitors as its script walks. `is_instance: false` gives
            // the non-instance class-nesting allowance, which a
            // template expression (always nested deeper) exceeds
            // either way.
            hooks: Some(ScriptRuleHooks {
                runes,
                is_instance: false,
                transpiled: false,
            }),
            in_reactive_expression: true,
            props_calls: 0,
            declarator_init_call: None,
            bindable_positions: Vec::new(),
            props_id_calls: 0,
            declarator_binds_identifier: false,
            field_init_call: None,
            constructor_assignment_call: None,
            statement_call: None,
            trace_slot: None,
            callee_span: None,
            template_root_pending: true,
            plain_function_depth: 0,
            node_spans: Vec::new(),
            state_fields: Vec::new(),
            in_constructor_body: false,
            reactive_statement: None,
        };
        if std::mem::take(&mut walker.tree.declaration_tag_init)
            && let Some(Statement::ExpressionStatement(es)) = parsed.program.body.first()
        {
            walker.declarator_init_call = direct_call_span(&es.expression);
        }
        let events_before = walker.tree.script_rule_events.len();
        for stmt in &parsed.program.body {
            walker.visit_stmt(stmt);
        }
        let template_events = self.script_rule_events.split_off(events_before);
        self.template_rule_events.extend(template_events);
        // Apply template flags to refs produced during that walk.
        // PendingRef doesn't yet carry template flags; set them on
        // the refs produced in this slice via a post-pass.
        apply_template_flags_since(
            &mut self.pending_refs,
            range,
            flags,
            true,
            ctx.in_control_flow,
        );
        drop(parsed);
        alloc.reset();
        self.expr_alloc = Some(alloc);
    }

    /// Declare each identifier in a binding pattern (e.g. the body of
    /// a `{@const NAME = EXPR}` left-hand side) into `each_scope`
    /// with `BindingKind::Each`. Rest-element-nested identifiers get
    /// `inside_rest = true`. Default-value expressions are walked in
    /// `parent_scope` so their references resolve to outer bindings.
    ///
    /// Only `walk_const_tag` calls this directly today —
    /// `declare_each_context` / `declare_snippet_params` /
    /// `declare_let_directive` retired in Phase 4 of
    /// `notes/PLAN-template-scope-unification.md` (the unified
    /// walker emits bindings via `enter_scope` instead).
    #[allow(clippy::too_many_arguments)]
    fn declare_each_pattern(
        &mut self,
        pat: &BindingPattern<'_>,
        each_scope: ScopeId,
        parent_scope: ScopeId,
        offset: i32,
        inside_rest: bool,
        source: &str,
        lang: svn_parser::document::ScriptLang,
    ) {
        // Pattern walking moved to `svn_analyze::template_scope` so
        // analyze and lint share a single primitive (round-3 F5,
        // round-4 G6/G9 each landed parallel fixes in both walkers
        // before unification). The helper returns ordered bindings
        // plus the source ranges of any `AssignmentPattern` defaults
        // — defaults walk in the PARENT scope here so a
        // `{ a = b }` default's `b` resolves to a parent binding,
        // not the just-declared `a`.
        let pb = svn_analyze::template_scope::collect_pattern_bindings(pat, offset);
        for b in &pb.bindings {
            let bid = self.declare(
                each_scope,
                b.name.clone(),
                b.range,
                BindingKind::Each,
                DeclarationKind::Const,
                InitialKind::EachBlock,
            );
            // Apply `inside_rest` as the OR of the caller-passed
            // baseline and the helper's per-binding flag — preserves
            // pre-helper behaviour where a caller could force-flag a
            // sub-tree (used by `declare_each_pattern` recursion's
            // own rest-element bookkeeping).
            self.bindings[bid.0 as usize].inside_rest = inside_rest || b.inside_rest;
        }
        for default_range in &pb.default_value_ranges {
            let mut ctx = TemplateCtx {
                source,
                scope: parent_scope,
                lang,
                in_control_flow: false,
            };
            self.walk_expr_range(*default_range, &mut ctx, RefFlags::default());
        }
    }

    /// `bind:foo={expr}` behaves like a write to `expr` from
    /// upstream's scope walker (scope.js `BindDirective` pushes to
    /// `updates`). Also captures the bind's BASE identifier (even
    /// when the expression is a member chain like `rest[0]`) onto
    /// the backing binding's `bind_reference_count`, for
    /// `bind_invalid_each_rest`.
    fn register_bind_update(&mut self, range: Range, directive: Range, ctx: &mut TemplateCtx<'_>) {
        let Some(raw) = ctx.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        if raw.trim().is_empty() {
            return;
        }
        // Parse the expression so a leading comment or a non-ASCII
        // name doesn't hide the identifier the bind roots on.
        let wrapped = format!("({raw});");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, &wrapped, ctx.lang);
        let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
            return;
        };
        let mut expr = &stmt.expression;
        while let Expression::ParenthesizedExpression(p) = expr {
            expr = &p.expression;
        }
        // The compiler strips TypeScript wrappers before it validates
        // the binding, so `bind:value={x as T}` binds to `x`.
        let mut target = expr;
        loop {
            target = match target {
                Expression::ParenthesizedExpression(p) => &p.expression,
                Expression::TSAsExpression(e) => &e.expression,
                Expression::TSSatisfiesExpression(e) => &e.expression,
                Expression::TSNonNullExpression(e) => &e.expression,
                Expression::TSTypeAssertion(e) => &e.expression,
                _ => break,
            };
        }
        if let Expression::Identifier(id) = target {
            self.pending_writes.push(PendingWrite {
                scope: ctx.scope,
                targets: vec![SmolStr::from(id.name.as_str())],
                bare: true,
                range: directive,
                is_binding: true,
                origin: WriteOrigin::Template,
            });
        }
        // Bare identifier → also push a reassignment for
        // `non_reactive_update`.
        if let Expression::Identifier(id) = expr {
            self.pending_updates.push(PendingUpdate {
                scope: ctx.scope,
                name: SmolStr::from(id.name.as_str()),
                range,
                is_reassign: true,
            });
        }
        // The base identifier of member chains counts too —
        // `rest[0]` and `rest.foo` both root on `rest`.
        if let Some((base, _, _)) = base_identifier(expr)
            && let Some(bid) = resolve_by_name(&self.scopes, ctx.scope, base)
        {
            self.bindings[bid.0 as usize].bind_reference_count += 1;
        }
    }

    fn record_template_ref(
        &mut self,
        name: &str,
        range: Range,
        ctx: &mut TemplateCtx<'_>,
        flags: RefFlags,
    ) {
        let depth = self.scopes[ctx.scope.0 as usize].function_depth;
        self.pending_refs.push(PendingRef {
            scope: ctx.scope,
            name: SmolStr::from(name),
            range,
            parent_kind: RefParentKind::Read,
            function_depth_at_use: depth,
            nested_in_state_call: false,
            in_function_closure: false,
            in_template: true,
            in_control_flow: ctx.in_control_flow,
            is_bind_this: flags.is_bind_this,
            parent_is_call: false,
            in_reactive_statement: false,
            ignored: None,
        });
    }

    fn build_script(
        &mut self,
        script: &ScriptSection<'_>,
        program: &Program<'_>,
        root_scope: ScopeId,
        runes: bool,
        leading_ignores: &[SmolStr],
    ) {
        self.build_script_inner(script, program, root_scope, false, runes, leading_ignores);
    }
}

/// Visitor mapping `TemplateScopeVisitor` calls into lint-side
/// `TreeBuilder` mutations. Mirrors what the pre-Phase-4
/// `walk_template_fragment` match arms did, broken out per node-kind:
/// the unified walker drives recursion + scope/control-flow
/// bracketing, the visitor does per-block expression walks and
/// binding declarations.
struct LintScopeVisitor<'a, 'src> {
    builder: &'a mut TreeBuilder,
    ctx: TemplateCtx<'src>,
    /// Parent scope ids saved by `enter_scope`, restored on
    /// `leave_scope`. Stack depth equals the number of currently-open
    /// child scopes the visitor is INSIDE (let-directive, each,
    /// snippet, await branches).
    scope_stack: Vec<ScopeId>,
    /// Saved `in_control_flow` flags pushed by `enter_control_flow`.
    /// `leave_control_flow` pops and restores so that nested control-
    /// flow blocks return to their outer state correctly (an outer
    /// `{#if}` with an inner `{#each}` should still see
    /// `in_control_flow=true` after the each closes).
    control_flow_stack: Vec<bool>,
}

impl<'src> svn_analyze::template_scope::TemplateScopeVisitor for LintScopeVisitor<'_, 'src> {
    // Lint answers the compiler's questions, so it needs the compiler's
    // scope tree rather than the overlay's.
    const COMPILER_SCOPES: bool = true;

    fn declare_in_current_scope(&mut self, bindings: &[svn_analyze::template_scope::BoundIdent]) {
        for b in bindings {
            let bid = self.builder.declare(
                self.ctx.scope,
                b.name.clone(),
                b.range,
                BindingKind::Template,
                DeclarationKind::Const,
                InitialKind::EachBlock,
            );
            self.builder.bindings[bid.0 as usize].inside_rest = b.inside_rest;
        }
    }

    /// The compiler declares a snippet's name in the scope enclosing
    /// the `{#snippet}` block (`scope.declare(node.expression, 'normal',
    /// 'function', node)`), so `{@render name()}` and other template
    /// references resolve to the snippet rather than to a same-named
    /// script binding.
    fn visit_snippet_block(&mut self, block: &svn_parser::SnippetBlock) {
        let Some(range) = snippet_name_range(block, self.ctx.source) else {
            return;
        };
        self.builder.declare(
            self.ctx.scope,
            block.name.clone(),
            range,
            BindingKind::Normal,
            DeclarationKind::Function,
            InitialKind::SnippetBlock,
        );
    }

    fn enter_scope(
        &mut self,
        kind: svn_analyze::template_scope::ScopeKind,
        bindings: &[svn_analyze::template_scope::BoundIdent],
        scope_range: svn_core::Range,
    ) {
        use svn_analyze::template_scope::ScopeKind;
        let current = self.ctx.scope;
        // A named-slot child's scope is a sibling of the component's
        // default scope: its parent is the scope the default scope was
        // opened from.
        let parent = match kind {
            ScopeKind::ComponentSlot => self.scope_stack.last().copied().unwrap_or(current),
            _ => current,
        };
        let child = match kind {
            // An element's children fragment is porous (`transparent`).
            ScopeKind::ElementFragment => self.builder.new_porous_scope(parent),
            _ => self.builder.new_scope(Some(parent)),
        };
        // Record the scope's lexical span so a reference inside it
        // resolves against this scope, not the whole-file declaration
        // set.
        self.builder.scopes[child.0 as usize].range = Some(scope_range);
        self.scope_stack.push(current);
        self.ctx.scope = child;

        // Per-kind binding declaration. Convention for `Each`:
        // bindings[..] = context entries, bindings[last] = index when
        // `has_index` is true. Index kind is `Static` (no key) or
        // `Template` (keyed) per upstream `scope.js`.
        //
        // Await-branch context bindings are `Template` per the
        // `BindingKind::Template` doc comment ("`{#await promise then
        // value}` / `{@const X = …}` / `<Foo let:x>`"). Declaring
        // them as `Each` lets each-specific rules
        // (`bind_invalid_each_rest`, etc.) misfire on `{:then
        // {...rest}}` / `{:catch {...rest}}` destructures.
        //
        // `let:` bindings (on elements and component default scopes)
        // are `Template`; the `{:then}` / `{:catch}` value scope
        // declares the pattern's names as plain bindings.
        let declare_kind = match kind {
            ScopeKind::Each { .. } => BindingKind::Each,
            ScopeKind::AwaitThen | ScopeKind::AwaitCatch => BindingKind::Template,
            ScopeKind::AwaitValue => BindingKind::Normal,
            ScopeKind::Snippet => BindingKind::Snippet,
            ScopeKind::LetDirective | ScopeKind::Element | ScopeKind::ComponentDefault => {
                BindingKind::Template
            }
            ScopeKind::Block | ScopeKind::ElementFragment | ScopeKind::ComponentSlot => {
                debug_assert!(bindings.is_empty(), "{kind:?} scopes declare nothing");
                BindingKind::Template
            }
            ScopeKind::Fragment => unreachable!("walker doesn't call enter_scope for Fragment"),
        };

        let context_count = match kind {
            ScopeKind::Each { has_index, .. } if has_index => bindings.len().saturating_sub(1),
            _ => bindings.len(),
        };
        let context_bindings = &bindings[..context_count];
        // Snippet parameters are the one template binding the compiler
        // declares with `let`.
        let declaration_kind = if matches!(kind, ScopeKind::Snippet) {
            DeclarationKind::Let
        } else {
            DeclarationKind::Const
        };
        for b in context_bindings {
            let bid = self.builder.declare(
                child,
                b.name.clone(),
                b.range,
                declare_kind,
                declaration_kind,
                InitialKind::EachBlock,
            );
            self.builder.bindings[bid.0 as usize].inside_rest = b.inside_rest;
            if matches!(kind, ScopeKind::AwaitThen | ScopeKind::AwaitCatch) {
                self.builder.late_declared.push(bid);
            }
        }
        if let ScopeKind::Each {
            has_index,
            is_keyed,
        } = kind
            && has_index
        {
            // Index binding: last entry. Kind is `Template` when
            // keyed, `Static` otherwise (matches upstream scope.js
            // and the pre-Phase-4 lint walker).
            let index = &bindings[bindings.len() - 1];
            let index_kind = if is_keyed {
                BindingKind::Template
            } else {
                BindingKind::Static
            };
            let had_error = self.builder.declaration_error.is_some();
            self.builder.declare(
                child,
                index.name.clone(),
                index.range,
                index_kind,
                DeclarationKind::Const,
                InitialKind::EachBlock,
            );
            // The compiler declares the index from a bare name with no
            // source position, so an error about it has none either.
            if !had_error && let Some(error) = &mut self.builder.declaration_error {
                error.2 = Range::new(0, 0);
            }
        }
    }

    fn leave_scope(&mut self, _kind: svn_analyze::template_scope::ScopeKind) {
        if let Some(parent) = self.scope_stack.pop() {
            self.ctx.scope = parent;
        }
    }

    fn enter_control_flow(&mut self) {
        self.control_flow_stack.push(self.ctx.in_control_flow);
        self.ctx.in_control_flow = true;
    }

    fn leave_control_flow(&mut self) {
        if let Some(saved) = self.control_flow_stack.pop() {
            self.ctx.in_control_flow = saved;
        }
    }

    fn visit_expr(&mut self, range: svn_core::Range) {
        self.builder
            .walk_expr_range(range, &mut self.ctx, RefFlags::default());
    }

    fn visit_element(&mut self, e: &svn_parser::Element) {
        // Plain DOM element: no component-name reference; just walk
        // attributes (skipping let:directives, which the walker's
        // enter_scope handles separately).
        self.walk_attrs_skipping_let(&e.attributes);
    }

    fn visit_component(&mut self, c: &svn_parser::Component) {
        // Record a reference for the component tag's first segment
        // so `export_let_unused` correctly sees the binding as
        // referenced (matches pre-Phase-4 `walk_element_like`).
        let first_seg = c.name.split('.').next().unwrap_or("");
        if !first_seg.is_empty() {
            self.builder.record_template_ref(
                first_seg,
                c.range,
                &mut self.ctx,
                RefFlags::default(),
            );
        }
        self.walk_attrs_skipping_let(&c.attributes);
    }

    fn visit_svelte_element(&mut self, s: &svn_parser::SvelteElement) {
        // `<svelte:self>` and friends record a reference under their
        // first identifier — but pre-Phase-4 `walk_element_like`
        // only records when `component_ref` is `Some(_)`, which
        // SvelteElement never passes. Preserve that — no ref here.
        self.walk_attrs_skipping_let(&s.attributes);
    }

    fn visit_at_const(&mut self, _bound_names: &[smol_str::SmolStr], expr_range: svn_core::Range) {
        // Lint re-parses the `{@const}` body as `let NAME = EXPR;`
        // (handles destructure forms like `{@const {a, b} = x}` that
        // the leading-identifier extractor would skip). The full
        // expression range is what `walk_const_tag` consumes.
        let mut ctx = self.ctx;
        self.builder.walk_const_tag(expr_range, &mut ctx);
        // walk_const_tag may declare bindings in current scope but
        // doesn't modify scope/in_control_flow — sync back any
        // changes (none expected, but keep symmetric).
        self.ctx.scope = ctx.scope;
        self.ctx.in_control_flow = ctx.in_control_flow;
    }
}

impl<'src> LintScopeVisitor<'_, 'src> {
    /// Walk every attribute except `let:` directives (which the
    /// walker's `enter_scope(LetDirective, …)` handles) and bind:foo
    /// pseudo-writes (which `walk_template_attr` records).
    fn walk_attrs_skipping_let(&mut self, attrs: &[svn_parser::ast::Attribute]) {
        use svn_parser::ast::{Attribute, DirectiveKind};
        for attr in attrs {
            if matches!(attr, Attribute::Directive(d) if d.kind == DirectiveKind::Let) {
                continue;
            }
            self.builder.walk_template_attr(attr, &mut self.ctx);
        }
    }
}

impl TreeBuilder {
    /// Instance-script variant — marks the walker so `$:` labels at
    /// the program top level flip `in_reactive_statement` on descent.
    /// Upstream guards the `reactive_declaration_module_script_dependency`
    /// check behind `ast_type === 'instance'`, so we gate the same way.
    fn build_script_as_instance(
        &mut self,
        script: &ScriptSection<'_>,
        program: &Program<'_>,
        root_scope: ScopeId,
        runes: bool,
        leading_ignores: &[SmolStr],
    ) {
        self.build_script_inner(script, program, root_scope, true, runes, leading_ignores);
    }

    /// `program` is the pre-parsed body of `script` — parsed once by
    /// `walk_parsed` and shared with the script-AST rules.
    fn build_script_inner(
        &mut self,
        script: &ScriptSection<'_>,
        program: &Program<'_>,
        root_scope: ScopeId,
        is_instance: bool,
        runes: bool,
        leading_ignores: &[SmolStr],
    ) {
        let base = script.content_range.start;
        let preprocess_ts = self.preprocess_ts;
        let start_depth = self.scopes[root_scope.0 as usize].function_depth;
        // Index every comment in the script body so the walker can
        // resolve leading `// svelte-ignore …` runs per node. Offsets
        // are script-local, matching oxc spans. Codes are parsed with
        // the file's real runes flag — upstream's extract_svelte_ignore
        // is strict in runes mode and lax (legacy dashed names accepted)
        // otherwise.
        let script_comments = crate::ignore::ScriptComments::build(
            program.comments.iter().map(|c| (c.span.start, c.span.end)),
            script.content,
            runes,
        );
        let mut walker = ScriptWalker {
            tree: self,
            base_offset: base,
            scope_stack: vec![root_scope],
            function_depth: start_depth,
            rune_bump: 0,
            in_function_closure: false,
            in_state_arg_nested: false,
            in_reactive_statement: false,
            is_instance,
            at_program_top: false,
            script_comments,
            script_content: script.content,
            ignore_frames: Vec::new(),
            counts_await: is_instance,
            hooks: Some(ScriptRuleHooks {
                runes,
                is_instance,
                transpiled: crate::rules::typescript_features::script_is_transpiled(
                    script,
                    preprocess_ts,
                ),
            }),
            in_reactive_expression: false,
            props_calls: 0,
            declarator_init_call: None,
            bindable_positions: Vec::new(),
            props_id_calls: 0,
            declarator_binds_identifier: false,
            field_init_call: None,
            constructor_assignment_call: None,
            statement_call: None,
            trace_slot: None,
            callee_span: None,
            template_root_pending: false,
            plain_function_depth: 0,
            node_spans: Vec::new(),
            state_fields: Vec::new(),
            in_constructor_body: false,
            reactive_statement: None,
        };
        // Push the template-comment ignores so they apply to every
        // reference recorded during this script walk.
        if !leading_ignores.is_empty() {
            walker.ignore_frames.push(leading_ignores.to_vec());
        }
        for directive in &program.directives {
            walker.string_literal_hook(&directive.expression);
        }
        for stmt in &program.body {
            walker.at_program_top = true;
            walker.visit_stmt(stmt);
        }
        if !leading_ignores.is_empty() {
            walker.ignore_frames.pop();
        }
        // `$: x = …` declares `x` at the instance root unless something
        // already declares it. Done after the walk, so a reference
        // recorded before the reactive statement still resolves to it
        // (references are resolved in `finish`).
        for (name, range) in std::mem::take(&mut self.implicit_reactive_decls) {
            if resolve_by_name(&self.scopes, root_scope, &name).is_some() {
                continue;
            }
            self.declare(
                root_scope,
                name,
                range,
                BindingKind::LegacyReactive,
                DeclarationKind::Let,
                InitialKind::None,
            );
        }
        // Harvest non-runes `export` facts from the instance script's
        // top level — so the later `promote_non_runes_exports` pass
        // reads owned `SmolStr` facts instead of re-parsing the entire
        // instance body. Names only; the resolve / `Var|Let` gate runs
        // later against the built tree. Mirrors the old re-parse's
        // statement matching exactly.
        if is_instance {
            for stmt in &program.body {
                // `export let x` (a Svelte 4 prop) and `export { x as
                // y }` (a rename) are separate statement kinds now.
                if let Statement::ExportDeclaration(end) = stmt {
                    let oxc_ast::ast::Declaration::VariableDeclaration(v) = &end.declaration else {
                        continue;
                    };
                    // `export const` doesn't become a prop.
                    if matches!(v.kind, oxc_ast::ast::VariableDeclarationKind::Const) {
                        continue;
                    }
                    for d in &v.declarations {
                        for name in idents_in_pattern(&d.id) {
                            self.nonrunes_export_idents.push(SmolStr::from(name));
                        }
                    }
                    continue;
                }
                {
                    let Statement::ExportNamedDeclaration(end) = stmt else {
                        continue;
                    };
                    for spec in &end.specifiers {
                        use oxc_ast::ast::ModuleExportName;
                        let local = match &spec.local {
                            ModuleExportName::IdentifierName(id) => id.name.as_str(),
                            ModuleExportName::IdentifierReference(id) => id.name.as_str(),
                            ModuleExportName::StringLiteral(l) => l.value.as_str(),
                        };
                        self.export_spec_locals.push(SmolStr::from(local));
                        // Legacy mode promotes only `export { a as b }`
                        // with identifier names on both sides.
                        let exported = match (&spec.local, &spec.exported) {
                            (ModuleExportName::StringLiteral(_), _)
                            | (_, ModuleExportName::StringLiteral(_)) => continue,
                            (_, ModuleExportName::IdentifierName(id)) => id.name.as_str(),
                            (_, ModuleExportName::IdentifierReference(id)) => id.name.as_str(),
                        };
                        let exported = Some(exported);
                        // Record the alias only when it differs from the
                        // local — matches the old `alias != local` gate.
                        let alias = exported.filter(|a| *a != local).map(SmolStr::from);
                        self.nonrunes_export_specs
                            .push((SmolStr::from(local), alias));
                    }
                }
            }
        }
    }

    fn finish(mut self, module_root: ScopeId, instance_root: ScopeId) -> ScopeTree {
        // Pass 2: drain updates first so `binding.reassigned` /
        // `mutated` are set before rules consult refs. Upstream
        // actually does references-first → updates-second, but the
        // order doesn't matter since neither pass mutates the other's
        // target.

        let mut unresolved: Vec<UnresolvedRef> = Vec::new();

        // resolve references (populate binding.references)
        for r in std::mem::take(&mut self.pending_refs) {
            if let Some(bid) = resolve_by_name(&self.scopes, r.scope, &r.name) {
                // Skip references that ARE the declaring identifier
                // itself — upstream's Identifier visitor bails in
                // `is_reference(node, parent)` for the declaration
                // site.
                let declared_at = self.bindings[bid.0 as usize].range;
                if declared_at.start == r.range.start && declared_at.end == r.range.end {
                    continue;
                }
                self.bindings[bid.0 as usize].references.push(Reference {
                    range: r.range,
                    parent_kind: r.parent_kind,
                    function_depth_at_use: r.function_depth_at_use,
                    nested_in_state_call: r.nested_in_state_call,
                    in_template: r.in_template,
                    in_control_flow: r.in_control_flow,
                    is_bind_this: r.is_bind_this,
                    in_function_closure: r.in_function_closure,
                    parent_is_call: r.parent_is_call,
                    in_reactive_statement: r.in_reactive_statement,
                    ignored: r.ignored.clone(),
                });
            } else {
                unresolved.push(UnresolvedRef {
                    name: r.name,
                    range: r.range,
                    scope: r.scope,
                    parent_is_call: r.parent_is_call,
                    ignored: r.ignored,
                });
            }
        }

        for u in std::mem::take(&mut self.pending_updates) {
            if let Some(bid) = resolve_by_name(&self.scopes, u.scope, &u.name) {
                let b = &mut self.bindings[bid.0 as usize];
                // Skip self-reference at declaration site.
                if b.range.start == u.range.start && b.range.end == u.range.end {
                    continue;
                }
                if u.is_reassign {
                    b.reassigned = true;
                } else {
                    b.mutated = true;
                }
            }
        }

        let pending_writes = std::mem::take(&mut self.pending_writes);
        let write_violations = resolve_writes(&self, pending_writes);

        let store_refs: Vec<StoreRef> = unresolved
            .iter()
            .filter(|r| {
                let n = r.name.as_bytes();
                n.first() == Some(&b'$') && n.len() > 1 && n[1] != b'$'
            })
            .map(|r| StoreRef {
                name: r.name.clone(),
                range: r.range,
                nested_store: resolve_by_name(&self.scopes, r.scope, &r.name[1..]).is_some_and(
                    |b| {
                        let scope = self.bindings[b.0 as usize].scope;
                        scope != module_root && scope != instance_root
                    },
                ),
                parent_is_call: r.parent_is_call,
            })
            .collect();
        synthesize_store_subs(&mut self, &mut unresolved, instance_root);

        ScopeTree {
            scopes: self.scopes,
            bindings: self.bindings,
            module_root,
            instance_root,
            unresolved_refs: unresolved,
            has_await: self.has_await,
            write_violations,
            custom_element_props_candidates: self.custom_element_props_candidates,
            custom_element_props_ignored: self.custom_element_props_ignored,
            nonrunes_export_idents: self.nonrunes_export_idents,
            nonrunes_export_specs: self.nonrunes_export_specs,
            export_spec_locals: self.export_spec_locals,
            script_rule_events: self.script_rule_events,
            declaration_error: self.declaration_error,
            store_refs,
            module_script_range: None,
            reactive_statements: self.reactive_statements,
            reactive_assignment_targets: self.reactive_assignment_targets.into_iter().collect(),
            template_rule_events: {
                let mut events = self.template_rule_events;
                events.sort_by_key(|e| e.range().start);
                events
            },
        }
    }
}

/// The identifiers of an assignment target that the compiler's
/// `validate_no_const_assignment` inspects: the identifier itself, or
/// the plain identifier leaves of array / object patterns. Defaults,
/// rest elements and member expressions are not inspected.
fn checked_write_targets(t: &AssignmentTarget<'_>, out: &mut Vec<SmolStr>) {
    use oxc_ast::ast::AssignmentTargetMaybeDefault as ATMD;
    use oxc_ast::ast::AssignmentTargetProperty as ATP;
    match t {
        AssignmentTarget::AssignmentTargetIdentifier(id) => {
            out.push(SmolStr::from(id.name.as_str()))
        }
        AssignmentTarget::ArrayAssignmentTarget(arr) => {
            for el in arr.elements.iter().flatten() {
                if !matches!(el, ATMD::AssignmentTargetWithDefault(_))
                    && let Some(target) = el.as_assignment_target()
                {
                    checked_write_targets(target, out);
                }
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(obj) => {
            for p in &obj.properties {
                match p {
                    ATP::AssignmentTargetPropertyIdentifier(pi) if pi.init.is_none() => {
                        out.push(SmolStr::from(pi.binding.name.as_str()));
                    }
                    ATP::AssignmentTargetPropertyIdentifier(_) => {}
                    ATP::AssignmentTargetPropertyProperty(pp) => {
                        if !matches!(pp.binding, ATMD::AssignmentTargetWithDefault(_))
                            && let Some(target) = pp.binding.as_assignment_target()
                        {
                            checked_write_targets(target, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The compiler's `validate_assignment` over every recorded write:
/// the first import or non-each `const` among the targets rejects the
/// write; otherwise a bare target that is an each-block item or a
/// snippet parameter does.
fn resolve_writes(tree: &TreeBuilder, writes: Vec<PendingWrite>) -> Vec<WriteViolation> {
    let mut out = Vec::new();
    for w in writes {
        let binding = |name: &SmolStr| {
            resolve_by_name(&tree.scopes, w.scope, name).map(|bid| &tree.bindings[bid.0 as usize])
        };
        let constant = w.targets.iter().filter_map(binding).find(|b| {
            b.declaration_kind == DeclarationKind::Import
                || (b.declaration_kind == DeclarationKind::Const && b.kind != BindingKind::Each)
        });
        let kind = match constant {
            Some(b) => Some(WriteViolationKind::Constant {
                import: b.declaration_kind == DeclarationKind::Import,
            }),
            None if w.bare => w
                .targets
                .first()
                .and_then(binding)
                .and_then(|b| match b.kind {
                    BindingKind::Each => Some(WriteViolationKind::EachItem),
                    BindingKind::Snippet => Some(WriteViolationKind::SnippetParameter),
                    _ => None,
                }),
            None => None,
        };
        if let Some(kind) = kind {
            out.push(WriteViolation {
                kind,
                range: w.range,
                is_binding: w.is_binding,
                origin: w.origin,
            });
        }
    }
    out
}

/// For each unresolved `$name` reference that would be a store
/// auto-subscription — i.e. name starts with `$`, isn't a reserved
/// `$$*` name, and there is a matching `name` binding in the
/// instance or module scope OR `$name` itself is a known rune — emit
/// a synthetic `StoreSub` binding in the instance scope and migrate
/// the references onto it. Mirrors upstream
/// `2-analyze/index.js:355-450`.
fn synthesize_store_subs(
    tree: &mut TreeBuilder,
    unresolved: &mut Vec<UnresolvedRef>,
    instance_root: ScopeId,
) {
    use std::collections::HashMap as StdMap;
    let mut buckets: StdMap<SmolStr, Vec<usize>> = StdMap::new();
    for (i, r) in unresolved.iter().enumerate() {
        let n = r.name.as_str();
        if !n.starts_with('$') {
            continue;
        }
        // `$` alone, or `$$*` (ambients / reserved) → skip.
        if n.len() == 1 || n.as_bytes().get(1).copied() == Some(b'$') {
            continue;
        }
        let store_name = &n[1..];
        // `instance.scope.get(store_name)` upstream: the instance scope
        // first, then the module scope it is a child of.
        let backing = resolve_by_name(&tree.scopes, instance_root, store_name);
        // No backing declaration → upstream leaves the reference in
        // `module.scope.references` (no store-sub synthesis). For
        // rune names that surviving reference is what flips the file
        // into runes mode.
        if backing.is_none() {
            continue;
        }
        // Upstream guards:
        //   `declaration && get_rune(init) !== null` → DON'T synthesize
        //   EXCEPT the `store_name !== 'props' && get_rune === '$props'`
        //   carve-out (which preserves e.g. `const foo = $props(); $foo()`
        //   as a conflict).
        if let Some(bid) = backing {
            let backing_binding = &tree.bindings[bid.0 as usize];
            // The compiler synthesizes store subscriptions before its
            // analyze pass rewires each `$props()` destructure binding's
            // `initial` to the property default, so what it inspects is
            // still the declarator's `$props()` call — for plain
            // (`let { a } = $props()`), `$bindable` (`let { a =
            // $bindable() } = $props()`) and rest bindings alike. Our
            // builder has already applied that rewire, so the binding
            // kinds that only a `$props()` destructure produces stand
            // in for the original initializer.
            let init_rune = match (&backing_binding.kind, &backing_binding.initial) {
                (BindingKind::Prop | BindingKind::BindableProp, _) => Some(RuneCall::Props),
                (_, InitialKind::RuneCall { rune, .. }) => Some(*rune),
                _ => None,
            };
            // Upstream guards: a rune-initialised declaration is not a
            // store, except that a `$props()` value named anything but
            // `props` still is (`const foo = $props(); $foo()`).
            // The rune-initialiser guard only applies to rune-named
            // subscriptions: every other `$name` is a subscription to
            // `name` whatever initialises it (`let { a } = $derived(x)`
            // still makes `$a` a store read).
            if is_rune_name(n)
                && let Some(rune) = init_rune
            {
                let props_exception = store_name != "props" && rune == RuneCall::Props;
                if !props_exception {
                    continue;
                }
            }
            // `import { derived } from 'svelte/store'` must not
            // capture `$derived` as a subscription — upstream skips
            // synthesis so the rune reference survives (flipping
            // runes) and `store_rune_conflict` stays silent on
            // `$derived(…)` calls (both verified against the
            // compiler).
            if n == "$derived"
                && let InitialKind::Import { source, .. } = &backing_binding.initial
                && source == "svelte/store"
            {
                continue;
            }
        }
        buckets.entry(SmolStr::from(n)).or_default().push(i);
    }
    // Stable order matters for deterministic diagnostics.
    let mut keys: Vec<&SmolStr> = buckets.keys().collect();
    keys.sort();
    for name in keys {
        let idxs = &buckets[name];
        // Declare the synthetic binding in the instance scope.
        let first = &unresolved[idxs[0]];
        let bid = tree.declare(
            instance_root,
            name.clone(),
            first.range,
            BindingKind::StoreSub,
            DeclarationKind::Synthetic,
            InitialKind::None,
        );
        // Move the unresolved refs into the synthetic binding's list.
        for &i in idxs {
            let r = &unresolved[i];
            tree.bindings[bid.0 as usize].references.push(Reference {
                range: r.range,
                parent_kind: RefParentKind::Read,
                function_depth_at_use: 0,
                nested_in_state_call: false,
                in_template: false,
                in_control_flow: false,
                is_bind_this: false,
                in_function_closure: false,
                parent_is_call: r.parent_is_call,
                in_reactive_statement: false,
                ignored: r.ignored.clone(),
            });
        }
    }
    // Drop the refs we moved. Walk in reverse to keep indices stable.
    let mut all_moved: Vec<usize> = buckets.values().flat_map(|v| v.iter().copied()).collect();
    all_moved.sort_unstable();
    for i in all_moved.into_iter().rev() {
        unresolved.swap_remove(i);
    }
}

/// `Some((byte_offset_of_token, token))` when `slice` consists of
/// exactly one ASCII identifier (`[A-Za-z_$][A-Za-z0-9_$]*`)
/// surrounded only by ASCII whitespace. Anything else — operators,
/// comments, unicode identifiers/whitespace — returns `None` so the
/// caller falls through to a real parse.
fn bare_identifier(slice: &str) -> Option<(usize, &str)> {
    let bytes = slice.as_bytes();
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
    if !(bytes[start].is_ascii_alphabetic() || matches!(bytes[start], b'_' | b'$')) {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'$'))
    {
        end += 1;
    }
    if bytes[end..].iter().all(|b| b.is_ascii_whitespace()) {
        Some((start, &slice[start..end]))
    } else {
        None
    }
}

/// Words that do NOT parse as a plain identifier reference in module
/// (strict-mode) code: ES keywords, literal keywords (`true`, `null`,
/// `this`, …), strict-mode reserved words, and module-context `await`.
/// The bare-identifier fast path must send these to the parser, which
/// records no reference for them.
fn is_reserved_word(token: &str) -> bool {
    matches!(
        token,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

/// The `validate_identifier_name` error for a declared name, if any:
/// `$` alone is `dollar_binding_invalid`, any other `$`-prefixed name
/// `dollar_prefix_invalid`.
pub(crate) fn dollar_name_error(name: &str) -> Option<(Code, String)> {
    if name == "$" {
        Some((
            Code::dollar_binding_invalid,
            crate::messages::dollar_binding_invalid(),
        ))
    } else if name.starts_with('$') {
        Some((
            Code::dollar_prefix_invalid,
            crate::messages::dollar_prefix_invalid(),
        ))
    } else {
        None
    }
}

/// Source range of a `{#snippet NAME(…)}` block's name: the identifier
/// following the `{#snippet` keyword.
fn snippet_name_range(block: &svn_parser::SnippetBlock, source: &str) -> Option<Range> {
    let text = source.get(block.range.start as usize..block.range.end as usize)?;
    let after_kw = text.find("#snippet")? + "#snippet".len();
    let name_off = after_kw + text[after_kw..].find(block.name.as_str())?;
    let start = block.range.start + name_off as u32;
    Some(Range::new(start, start + block.name.len() as u32))
}

/// A TypeScript-only function statement: `declare function f(): T` or
/// a bodiless overload signature. Both are ESTree `TSDeclareFunction`
/// nodes, which the compiler deletes before analysis.
fn is_ts_declare_function(f: &oxc_ast::ast::Function<'_>) -> bool {
    f.declare || f.body.is_none()
}

/// The identifier at the root of a member chain (`a` of `a.b[c].d`).
fn member_root_identifier<'e, 'a>(e: &'e Expression<'a>) -> Option<&'e IdentifierReference<'a>> {
    match e {
        Expression::Identifier(id) => Some(id),
        other => other
            .as_member_expression()
            .and_then(|m| member_root_identifier(m.object())),
    }
}

/// `this.name`, `this.#name` or `this[<literal>]` — an assignment
/// target that names a class field.
fn is_this_field_target(t: &AssignmentTarget<'_>) -> bool {
    match t {
        AssignmentTarget::StaticMemberExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
        }
        AssignmentTarget::PrivateFieldExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
        }
        AssignmentTarget::ComputedMemberExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
                && matches!(
                    m.expression,
                    Expression::StringLiteral(_)
                        | Expression::NumericLiteral(_)
                        | Expression::BooleanLiteral(_)
                        | Expression::NullLiteral(_)
                        | Expression::BigIntLiteral(_)
                        | Expression::RegExpLiteral(_)
                )
        }
        _ => false,
    }
}

/// The compiler's `get_name` for a class member key: an identifier's
/// name, `#name` for a private name, a literal's string value.
fn class_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::PrivateIdentifier(id) => Some(format!("#{}", id.name)),
        PropertyKey::StringLiteral(l) => Some(l.value.to_string()),
        PropertyKey::NumericLiteral(n) => Some(js_number_string(n.value)),
        _ => None,
    }
}

/// `String(value)` of a literal used as a computed member key.
fn literal_key_name(e: &Expression<'_>) -> Option<String> {
    match e {
        Expression::StringLiteral(l) => Some(l.value.to_string()),
        Expression::NumericLiteral(n) => Some(js_number_string(n.value)),
        Expression::BooleanLiteral(b) => Some(b.value.to_string()),
        Expression::NullLiteral(_) => Some("null".to_string()),
        _ => None,
    }
}

/// A number the way JavaScript's `String(n)` prints the common cases.
fn js_number_string(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// The field name `this.name` / `this.#name` / `this[<literal>]` writes.
fn this_field_name(t: &AssignmentTarget<'_>) -> Option<String> {
    match t {
        AssignmentTarget::StaticMemberExpression(m) => Some(m.property.name.to_string()),
        AssignmentTarget::PrivateFieldExpression(m) => Some(format!("#{}", m.field.name)),
        AssignmentTarget::ComputedMemberExpression(m) => literal_key_name(&m.expression),
        _ => None,
    }
}

/// A member of `this` as an assignment target.
fn is_this_member_target(t: &AssignmentTarget<'_>) -> bool {
    match t {
        AssignmentTarget::StaticMemberExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
        }
        AssignmentTarget::PrivateFieldExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
        }
        AssignmentTarget::ComputedMemberExpression(m) => {
            matches!(m.object, Expression::ThisExpression(_))
        }
        _ => false,
    }
}

/// The span of `e` when it is a call, once parentheses and TypeScript
/// wrappers (which the compiler's AST does not have) are removed.
fn direct_call_span(e: &Expression<'_>) -> Option<(u32, u32)> {
    match unwrap_ts_wrappers(e) {
        Expression::CallExpression(c) => Some((c.span.start, c.span.end)),
        _ => None,
    }
}

fn resolve_by_name(scopes: &[Scope], from: ScopeId, name: &str) -> Option<BindingId> {
    let mut cur = Some(from);
    while let Some(sid) = cur {
        let s = &scopes[sid.0 as usize];
        if let Some(&bid) = s.declarations.get(name) {
            return Some(bid);
        }
        cur = s.parent;
    }
    None
}

struct ScriptWalker<'b, 'src> {
    tree: &'b mut TreeBuilder,
    base_offset: u32,
    /// Stack of scopes; top is current.
    scope_stack: Vec<ScopeId>,
    function_depth: u32,
    /// Analyze-phase bump for refs visited inside `$derived(...)`/
    /// `$inspect(...)` arguments. Upstream does this in
    /// `CallExpression.js:244-262` — NOT in the scope walker. We fold
    /// it in to keep rule logic simple.
    rune_bump: u32,
    /// True when walking inside a FunctionDeclaration / FunctionExpression
    /// / ArrowFunctionExpression body. Non_reactive_update filters
    /// references by this flag.
    in_function_closure: bool,
    /// True when we're walking under a `$state(…)` or `$state.raw(…)`
    /// argument AT LEAST ONE level deep (i.e. the reference is nested,
    /// not the direct arg identifier). Used by
    /// `state_referenced_locally` to pick the "derived" vs "closure"
    /// message.
    in_state_arg_nested: bool,
    /// True when walking inside a top-level instance-script `$:`
    /// reactive statement (labeled with `$`). Refs recorded below
    /// here drive `reactive_declaration_module_script_dependency`.
    in_reactive_statement: bool,
    /// True for the instance-script walk only. Upstream guards the
    /// `reactive_declaration_module_script_dependency` trigger
    /// behind `ast_type === 'instance'`; we mirror it.
    is_instance: bool,
    /// True only while the CURRENT statement is a direct child of
    /// the Program body — upstream recognizes a `$:` reactive
    /// statement by its parent being Program, so a bare block at
    /// depth 0 does NOT count.
    at_program_top: bool,
    /// Index of every comment in the script body (built from
    /// `parsed.program.comments` before the walk starts) — resolves
    /// the leading `// svelte-ignore …` run for any node position.
    /// Offsets are script-local (oxc's span origin = script content
    /// start).
    script_comments: crate::ignore::ScriptComments,
    /// Script source text (= `ScriptSection::content`) — needed so
    /// the leading-comment lookup can verify gaps and same-line
    /// trailing positions against the raw bytes.
    script_content: &'src str,
    /// Live stack of ignore-code sets — one frame per node we
    /// entered that had leading `// svelte-ignore` comments. Active
    /// codes at any time = flatten all frames. Snapshot is cloned
    /// onto each `PendingRef` we record.
    ignore_frames: Vec<Vec<SmolStr>>,
    /// Whether a function-free `await` in this walk contributes to
    /// [`ScopeTree::has_await`] — true for the instance-script walk
    /// and template expression walks, false for the module script
    /// (upstream consults only `has_await || instance.has_await`).
    counts_await: bool,
    /// Script-AST rule callbacks riding this walk — `Some` for the
    /// module / instance script walks, `None` for template
    /// mini-expression walks (the rules are scoped to `<script>`
    /// bodies and do not fire inside template `{…}` expressions).
    /// Hook sites buffer events into `tree.script_rule_events`;
    /// nothing is emitted during the walk.
    hooks: Option<ScriptRuleHooks>,
    /// True while walking an expression whose `await`s suspend
    /// rendering: a template expression or a `$derived(…)` argument,
    /// up to the next function boundary (the compiler's
    /// `state.expression`).
    in_reactive_expression: bool,
    /// `$props()` calls seen so far in this walk (the compiler's
    /// `has_props_rune`).
    props_calls: u32,
    /// Span start of the call expression that is the direct
    /// initializer of the declarator being walked — the only
    /// position a `$props()` may take.
    declarator_init_call: Option<(u32, u32)>,
    /// Span starts of the `$bindable()` calls sitting directly as a
    /// property default of a `$props()` destructure — the only
    /// position a `$bindable()` may take.
    bindable_positions: Vec<(u32, u32)>,
    /// `$props.id()` calls seen so far in this walk.
    props_id_calls: u32,
    /// Whether the declarator owning `declarator_init_call` binds a
    /// plain identifier (the only pattern `$props.id()` accepts).
    declarator_binds_identifier: bool,
    /// The call that is the value of the non-static, non-computed
    /// class field being walked.
    field_init_call: Option<(u32, u32)>,
    /// The call assigned by a `this.<field> = …` statement directly in
    /// a constructor body.
    constructor_assignment_call: Option<(u32, u32)>,
    /// The call that is the whole expression of the statement being
    /// walked.
    statement_call: Option<(u32, u32)>,
    /// The call forming the first statement of the function body being
    /// walked, and whether that function is a generator — the one
    /// place `$inspect.trace()` may sit.
    trace_slot: Option<(u32, u32, bool)>,
    /// The callee of the call being walked (parentheses and TypeScript
    /// wrappers removed), and the call.
    callee_span: Option<((u32, u32), (u32, u32))>,
    /// A template expression is parsed as a one-statement program;
    /// that statement is not an expression statement to the compiler,
    /// which sees the bare expression. Set until the walk passes it.
    template_root_pending: bool,
    /// Function declarations / expressions (not arrows) enclosing the
    /// node being walked — outside all of them, `arguments` is an
    /// error.
    plain_function_depth: u32,
    /// The state fields of the innermost class body being walked (runes
    /// mode): name, span start of the declaring node, and whether a
    /// constructor assignment declares it.
    state_fields: Vec<(SmolStr, u32, bool)>,
    /// Walking a constructor body, outside any nested function.
    in_constructor_body: bool,
    /// The index (in `tree.reactive_statements`) of the top-level `$:`
    /// statement being walked.
    reactive_statement: Option<usize>,
    /// Spans of the statements, declarators and expressions enclosing
    /// the node being walked, innermost last — the compiler's
    /// `context.path` as far as its errors report a parent node.
    /// Parentheses and TypeScript wrappers are skipped, as the
    /// compiler's AST has neither.
    node_spans: Vec<(u32, u32)>,
}

impl<'b, 'src> ScriptWalker<'b, 'src> {
    fn cur_scope(&self) -> ScopeId {
        // Invariant: scope_stack is seeded with `root_scope` in
        // `build_script` and every push is paired with a pop.
        self.scope_stack.last().copied().unwrap_or(ScopeId(0))
    }

    fn abs(&self, start: u32, end: u32) -> Range {
        Range::new(start + self.base_offset, end + self.base_offset)
    }

    /// A string literal outside expression position (a property key,
    /// a module source, a directive), which the compiler's `Literal`
    /// visitor tests all the same.
    fn string_literal_hook(&mut self, lit: &oxc_ast::ast::StringLiteral<'_>) {
        if let Some(h) = self.hooks {
            let range = self.abs(lit.span.start, lit.span.end);
            h.string_literal(
                &mut self.tree.script_rule_events,
                &self.ignore_frames,
                self.tree.bidi_warned.as_ref(),
                &lit.value,
                range,
            );
        }
    }

    /// A property key: a computed key is an expression, a plain string
    /// key a literal.
    fn visit_key(&mut self, key: &PropertyKey<'_>, computed: bool) {
        if computed {
            if let Some(k) = expression_from_property_key(key) {
                self.visit_expr(k);
            }
        } else if let PropertyKey::StringLiteral(lit) = key {
            self.string_literal_hook(lit);
        }
    }

    fn write_origin(&self) -> WriteOrigin {
        match &self.hooks {
            Some(h) if h.is_instance => WriteOrigin::Instance,
            Some(_) => WriteOrigin::Module,
            None => WriteOrigin::Template,
        }
    }

    fn push_write(&mut self, targets: Vec<SmolStr>, bare: bool, span: oxc_span::Span) {
        if targets.is_empty() {
            return;
        }
        let write = PendingWrite {
            scope: self.cur_scope(),
            targets,
            bare,
            range: self.abs(span.start, span.end),
            is_binding: false,
            origin: self.write_origin(),
        };
        self.tree.pending_writes.push(write);
    }

    fn with_scope<F, R>(&mut self, scope: ScopeId, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.scope_stack.push(scope);
        let r = f(self);
        self.scope_stack.pop();
        r
    }

    fn with_function<F>(&mut self, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let prev_depth = self.function_depth;
        let prev_closure = self.in_function_closure;
        let prev_reactive = std::mem::replace(&mut self.in_reactive_expression, false);
        let prev_constructor = std::mem::replace(&mut self.in_constructor_body, false);
        self.function_depth += 1;
        self.in_function_closure = true;
        // Open a fresh non-porous scope for the function body so
        // params + body-locals don't leak back into the enclosing
        // scope. Upstream `scope.js` creates a child scope per
        // function; we were only bumping function_depth, which
        // caused parameter names (e.g. a `state` param on an
        // instance-script `function updateState(state)`) to become
        // instance-scope bindings — flipping spurious
        // `store_rune_conflict` fires on every other `$state()`
        // call in the same file.
        let parent_scope = self.cur_scope();
        let fn_scope = self.tree.new_scope(Some(parent_scope));
        self.scope_stack.push(fn_scope);
        f(self);
        self.scope_stack.pop();
        self.function_depth = prev_depth;
        self.in_function_closure = prev_closure;
        self.in_reactive_expression = prev_reactive;
        self.in_constructor_body = prev_constructor;
    }

    /// The statements of a function body. Its first statement is the
    /// one place a `$inspect.trace()` call may stand; `plain` marks a
    /// function declaration / expression (not an arrow), inside which
    /// `arguments` is the function's own.
    fn visit_function_body(&mut self, body: &FunctionBody<'_>, generator: bool, plain: bool) {
        // A directive prologue is the body's first statement to the
        // compiler.
        let slot = match body.statements.first() {
            Some(Statement::ExpressionStatement(es)) if body.directives.is_empty() => {
                direct_call_span(&es.expression).map(|(start, end)| (start, end, generator))
            }
            _ => None,
        };
        let prev = std::mem::replace(&mut self.trace_slot, slot);
        self.plain_function_depth += u32::from(plain);
        for s in &body.statements {
            self.visit_stmt(s);
        }
        self.plain_function_depth -= u32::from(plain);
        self.trace_slot = prev;
    }

    /// A constructor body: a statement `this.<field> = <call>` directly
    /// in it may create a state field with a rune call.
    fn visit_constructor_body(&mut self, body: &FunctionBody<'_>) {
        self.plain_function_depth += 1;
        self.in_constructor_body = true;
        for s in &body.statements {
            let call = match s {
                Statement::ExpressionStatement(es) => match unwrap_ts_wrappers(&es.expression) {
                    Expression::AssignmentExpression(a)
                        if a.operator == oxc_syntax::operator::AssignmentOperator::Assign
                            && is_this_field_target(&a.left) =>
                    {
                        direct_call_span(&a.right)
                    }
                    _ => None,
                },
                _ => None,
            };
            let prev = std::mem::replace(&mut self.constructor_assignment_call, call);
            self.visit_stmt(s);
            self.constructor_assignment_call = prev;
        }
        self.in_constructor_body = false;
        self.plain_function_depth -= 1;
    }

    fn visit_stmt(&mut self, stmt: &Statement<'_>) {
        let pushed = self.push_leading_ignores(Some(stmt.span().start));
        // Only the Program-body loop sets the flag; every statement
        // visited from here down is nested.
        let at_program_top = std::mem::replace(&mut self.at_program_top, false);
        let span = stmt.span();
        self.node_spans.push((span.start, span.end));
        self.visit_stmt_inner(stmt, at_program_top);
        self.node_spans.pop();
        if pushed {
            self.ignore_frames.pop();
        }
    }

    /// Resolve the `// svelte-ignore …` run leading a node that
    /// starts at `node_start` (upstream's leadingComments semantic —
    /// see [`crate::ignore::ScriptComments`]) and push it as a new
    /// ignore frame. Returns `true` if a frame was pushed so the
    /// caller pops on exit.
    fn push_leading_ignores(&mut self, node_start: Option<u32>) -> bool {
        if !self.script_comments.has_ignores() {
            return false;
        }
        let Some(start) = node_start else {
            return false;
        };
        let codes = self
            .script_comments
            .leading_ignores(self.script_content, start);
        if codes.is_empty() {
            false
        } else {
            self.ignore_frames.push(codes);
            true
        }
    }

    /// Declare a binding from the script walk, stamping the active
    /// ignore-frame snapshot onto it — upstream's `ignore_map` entry
    /// for the declaring node. Declaration-anchored rules
    /// (`non_reactive_update`, `export_let_unused`) read it to honor
    /// leading `// svelte-ignore` comments on the declaration.
    fn declare_with_ignores(
        &mut self,
        scope: ScopeId,
        name: SmolStr,
        range: Range,
        kind: BindingKind,
        declaration_kind: DeclarationKind,
        initial: InitialKind,
    ) -> BindingId {
        let ignored = self.current_ignore_snapshot();
        let id = self
            .tree
            .declare(scope, name, range, kind, declaration_kind, initial);
        self.tree.bindings[id.0 as usize].ignored = ignored;
        id
    }

    /// Flatten the active ignore-frames into a single snapshot vec.
    /// Returns `None` when no frames are active (the common case) so
    /// `PendingRef::ignored` stays cheap.
    fn current_ignore_snapshot(&self) -> Option<Vec<SmolStr>> {
        if self.ignore_frames.is_empty() {
            return None;
        }
        let mut out: Vec<SmolStr> = Vec::new();
        for frame in &self.ignore_frames {
            for c in frame {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        Some(out)
    }

    fn visit_stmt_inner(&mut self, stmt: &Statement<'_>, at_program_top: bool) {
        match stmt {
            Statement::VariableDeclaration(vd) => self.visit_var_decl(vd),
            // TypeScript-only statements (`declare function`, overload
            // signatures) are deleted by the compiler's
            // `remove_typescript_nodes` before any scope is built.
            Statement::FunctionDeclaration(f) if is_ts_declare_function(f) => {}
            Statement::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    self.declare_with_ignores(
                        self.cur_scope(),
                        SmolStr::from(id.name.as_str()),
                        self.abs(id.span.start, id.span.end),
                        BindingKind::Normal,
                        DeclarationKind::Function,
                        InitialKind::FunctionDecl,
                    );
                }
                self.with_function(|w| {
                    for p in &f.params.items {
                        w.declare_pattern(&p.pattern, DeclarationKind::Param);
                    }
                    if let Some(rest) = &f.params.rest {
                        w.declare_pattern(&rest.rest.argument, DeclarationKind::RestParam);
                    }
                    if let Some(body) = &f.body {
                        w.visit_function_body(body, f.generator, true);
                    }
                });
            }
            Statement::ClassDeclaration(cls) => self.visit_class_decl(cls),
            // `import type …` never reaches the compiler's scope builder.
            Statement::ImportDeclaration(imp) if imp.import_kind.is_type() => {}
            Statement::ImportDeclaration(imp) => {
                self.check_runes_import(imp);
                let source = SmolStr::from(imp.source.value.as_str());
                if let Some(specs) = &imp.specifiers {
                    for s in specs {
                        use oxc_ast::ast::ImportDeclarationSpecifier as S;
                        let (name, span, is_default) = match s {
                            // `import { type x }` — the specifier is
                            // filtered out before analysis.
                            S::ImportSpecifier(s) if s.import_kind.is_type() => continue,
                            S::ImportSpecifier(s) => (s.local.name.as_str(), s.local.span, false),
                            S::ImportDefaultSpecifier(s) => {
                                (s.local.name.as_str(), s.local.span, true)
                            }
                            S::ImportNamespaceSpecifier(s) => {
                                (s.local.name.as_str(), s.local.span, false)
                            }
                        };
                        self.declare_with_ignores(
                            self.cur_scope(),
                            SmolStr::from(name),
                            self.abs(span.start, span.end),
                            BindingKind::Normal,
                            DeclarationKind::Import,
                            InitialKind::Import {
                                source: source.clone(),
                                is_default,
                            },
                        );
                    }
                }
                self.string_literal_hook(&imp.source);
            }
            Statement::ExportDeclaration(end) => {
                {
                    let decl = &end.declaration;
                    // Re-wrap as a statement-like visit.
                    use oxc_ast::ast::Declaration;
                    match decl {
                        Declaration::VariableDeclaration(v) => self.visit_var_decl(v),
                        Declaration::FunctionDeclaration(f) if is_ts_declare_function(f) => {}
                        Declaration::FunctionDeclaration(f) => {
                            if let Some(id) = &f.id {
                                self.declare_with_ignores(
                                    self.cur_scope(),
                                    SmolStr::from(id.name.as_str()),
                                    self.abs(id.span.start, id.span.end),
                                    BindingKind::Normal,
                                    DeclarationKind::Function,
                                    InitialKind::FunctionDecl,
                                );
                            }
                            self.with_function(|w| {
                                for p in &f.params.items {
                                    w.declare_pattern(&p.pattern, DeclarationKind::Param);
                                }
                                if let Some(rest) = &f.params.rest {
                                    w.declare_pattern(
                                        &rest.rest.argument,
                                        DeclarationKind::RestParam,
                                    );
                                }
                                if let Some(body) = &f.body {
                                    w.visit_function_body(body, f.generator, true);
                                }
                            });
                        }
                        Declaration::ClassDeclaration(cls) => self.visit_class_decl(cls),
                        _ => {}
                    }
                    // `ExportNamedDeclaration.js` (after visiting the
                    // declaration): `export let` is legacy syntax in a
                    // runes-mode instance script.
                    if let Some(h) = self.hooks
                        && h.runes
                        && h.is_instance
                        && let Declaration::VariableDeclaration(v) = decl
                        && !v.declare
                        && v.kind == oxc_ast::ast::VariableDeclarationKind::Let
                    {
                        let range = self.abs(end.span.start, end.span.end);
                        self.push_error(
                            Code::legacy_export_invalid,
                            crate::messages::legacy_export_invalid(),
                            range,
                        );
                    }
                    if let Declaration::VariableDeclaration(v) = decl
                        && !v.declare
                    {
                        let range = self.abs(end.span.start, end.span.end);
                        for d in &v.declarations {
                            for id in crate::scope_util::binding_idents_in_pattern(&d.id) {
                                self.check_export(id.name.as_str(), range);
                            }
                        }
                    }
                }
            }
            Statement::ExportNamedDeclaration(end) if !end.export_kind.is_type() => {
                self.check_export_specifiers(&end.specifiers);
                self.check_default_export_specifiers(&end.specifiers, end.span);
            }
            Statement::ExportFromDeclaration(end) if !end.export_kind.is_type() => {
                self.check_export_specifiers(&end.specifiers);
                self.check_default_export_specifiers(&end.specifiers, end.span);
                self.string_literal_hook(&end.source);
            }
            Statement::ExportAllDeclaration(all) if !all.export_kind.is_type() => {
                self.string_literal_hook(&all.source);
            }
            Statement::BlockStatement(b) => {
                // Non-function block — porous w.r.t. function_depth.
                let s = self.tree.new_porous_scope(self.cur_scope());
                self.with_scope(s, |w| {
                    for stmt in &b.body {
                        w.visit_stmt(stmt);
                    }
                });
            }
            Statement::IfStatement(i) => {
                self.visit_expr(&i.test);
                self.visit_stmt(&i.consequent);
                if let Some(alt) = &i.alternate {
                    self.visit_stmt(alt);
                }
            }
            Statement::ForStatement(f) => {
                // Upstream gives every for-statement a porous block
                // scope (`create_block_scope`), so an init declaration
                // shadows outer bindings instead of overwriting them.
                let s = self.tree.new_porous_scope(self.cur_scope());
                self.with_scope(s, |w| {
                    if let Some(init) = &f.init {
                        match init {
                            ForStatementInit::VariableDeclaration(v) => w.visit_var_decl(v),
                            e => {
                                if let Some(expr) = expression_from_for_init(e) {
                                    w.visit_expr(expr);
                                }
                            }
                        }
                    }
                    if let Some(t) = &f.test {
                        w.visit_expr(t);
                    }
                    if let Some(u) = &f.update {
                        w.visit_expr(u);
                    }
                    w.visit_stmt(&f.body);
                });
            }
            Statement::ForInStatement(f) => {
                self.visit_for_in_of(&f.left, &f.right, &f.body);
            }
            Statement::ForOfStatement(f) => {
                self.visit_for_in_of(&f.left, &f.right, &f.body);
            }
            Statement::WhileStatement(w) => {
                self.visit_expr(&w.test);
                self.visit_stmt(&w.body);
            }
            Statement::DoWhileStatement(d) => {
                self.visit_stmt(&d.body);
                self.visit_expr(&d.test);
            }
            Statement::TryStatement(t) => {
                // The `try` and `finally` bodies are plain block
                // statements, so each gets its own porous block scope
                // (upstream `BlockStatement` → `create_block_scope`).
                let s = self.tree.new_porous_scope(self.cur_scope());
                self.with_scope(s, |w| {
                    for s in &t.block.body {
                        w.visit_stmt(s);
                    }
                });
                if let Some(h) = &t.handler {
                    // Catch params live in a porous scope covering the
                    // handler (upstream `CatchClause` declares them
                    // 'normal'/'let' in a `child(true)` scope) — body
                    // refs to the param must shadow outer bindings.
                    let s = self.tree.new_porous_scope(self.cur_scope());
                    self.with_scope(s, |w| {
                        if let Some(param) = &h.param {
                            w.declare_pattern(&param.pattern, DeclarationKind::Let);
                        }
                        for s in &h.body.body {
                            w.visit_stmt(s);
                        }
                    });
                }
                if let Some(f) = &t.finalizer {
                    let s = self.tree.new_porous_scope(self.cur_scope());
                    self.with_scope(s, |w| {
                        for s in &f.body {
                            w.visit_stmt(s);
                        }
                    });
                }
            }
            Statement::SwitchStatement(s) => {
                self.visit_expr(&s.discriminant);
                // Upstream: `SwitchStatement: create_block_scope` —
                // case-body declarations scope to the switch.
                let sc = self.tree.new_porous_scope(self.cur_scope());
                self.with_scope(sc, |w| {
                    for case in &s.cases {
                        if let Some(t) = &case.test {
                            w.visit_expr(t);
                        }
                        for s in &case.consequent {
                            w.visit_stmt(s);
                        }
                    }
                });
            }
            Statement::ExpressionStatement(es) => {
                if let Some(h) = self.hooks {
                    h.expression_statement(
                        &mut self.tree.script_rule_events,
                        &self.ignore_frames,
                        &es.expression,
                        self.base_offset,
                    );
                }
                let is_statement = !std::mem::replace(&mut self.template_root_pending, false);
                let call = is_statement
                    .then(|| direct_call_span(&es.expression))
                    .flatten();
                let prev = std::mem::replace(&mut self.statement_call, call);
                self.visit_expr(&es.expression);
                self.statement_call = prev;
            }
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.visit_expr(arg);
                }
            }
            Statement::LabeledStatement(lbl) => self.visit_labeled(lbl, at_program_top),
            Statement::ThrowStatement(t) => self.visit_expr(&t.argument),
            Statement::ExportDefaultDeclaration(ed) => {
                // `ExportDefaultDeclaration.js`: a component script may
                // not default-export anything.
                if self.hooks.is_some() {
                    let range = self.abs(ed.span.start, ed.span.end);
                    self.push_error(
                        Code::module_illegal_default_export,
                        crate::messages::module_illegal_default_export(),
                        range,
                    );
                }
                use oxc_ast::ast::ExportDefaultDeclarationKind as Ed;
                match &ed.declaration {
                    Ed::FunctionDeclaration(f) => {
                        self.with_function(|w| {
                            for p in &f.params.items {
                                w.declare_pattern(&p.pattern, DeclarationKind::Param);
                            }
                            if let Some(rest) = &f.params.rest {
                                w.declare_pattern(&rest.rest.argument, DeclarationKind::RestParam);
                            }
                            if let Some(body) = &f.body {
                                w.visit_function_body(body, f.generator, true);
                            }
                        });
                    }
                    Ed::ClassDeclaration(c) => self.visit_class_decl(c),
                    e => {
                        if let Some(expr) = expression_from_default(e) {
                            self.visit_expr(expr);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `ImportDeclaration.js` (runes mode): `svelte/internal` is off
    /// limits, and so are the legacy lifecycle functions.
    fn check_runes_import(&mut self, imp: &oxc_ast::ast::ImportDeclaration<'_>) {
        if !self.hooks.is_some_and(|h| h.runes) {
            return;
        }
        let source = imp.source.value.as_str();
        if source.starts_with("svelte/internal") {
            let range = self.abs(imp.span.start, imp.span.end);
            self.push_error(
                Code::import_svelte_internal_forbidden,
                crate::messages::import_svelte_internal_forbidden(),
                range,
            );
        }
        if source == "svelte" {
            for spec in imp.specifiers.iter().flatten() {
                use oxc_ast::ast::{ImportDeclarationSpecifier as S, ModuleExportName};
                let S::ImportSpecifier(spec) = spec else {
                    continue;
                };
                if spec.import_kind.is_type() {
                    continue;
                }
                let ModuleExportName::IdentifierName(imported) = &spec.imported else {
                    continue;
                };
                let name = imported.name.as_str();
                if matches!(name, "beforeUpdate" | "afterUpdate") {
                    let range = self.abs(spec.span.start, spec.span.end);
                    self.push_error(
                        Code::runes_mode_invalid_import,
                        crate::messages::runes_mode_invalid_import(name),
                        range,
                    );
                }
            }
        }
    }

    /// `validate_export` (runes mode): derived state, and state the
    /// component reassigns, may not be exported. Which one `name` is
    /// waits for the finished tree.
    fn check_export(&mut self, name: &str, range: Range) {
        if !self.hooks.is_some_and(|h| h.runes) {
            return;
        }
        let scope = self.cur_scope();
        self.push_gated_error(
            ErrorGate::DerivedExport {
                name: SmolStr::from(name),
                scope,
            },
            Code::derived_invalid_export,
            crate::messages::derived_invalid_export(),
            range,
        );
        self.push_gated_error(
            ErrorGate::ReassignedStateExport {
                name: SmolStr::from(name),
                scope,
            },
            Code::state_invalid_export,
            crate::messages::state_invalid_export(),
            range,
        );
    }

    /// `ExportSpecifier.js`: outside the instance script, each exported
    /// local goes through `validate_export`.
    fn check_export_specifiers(&mut self, specifiers: &[oxc_ast::ast::ExportSpecifier<'_>]) {
        use oxc_ast::ast::ModuleExportName;
        if self.is_instance || self.hooks.is_none() {
            return;
        }
        for spec in specifiers {
            if spec.export_kind.is_type() {
                continue;
            }
            let local = match &spec.local {
                ModuleExportName::IdentifierName(id) => id.name.as_str(),
                ModuleExportName::IdentifierReference(id) => id.name.as_str(),
                ModuleExportName::StringLiteral(l) => l.value.as_str(),
            };
            let range = self.abs(spec.span.start, spec.span.end);
            self.check_export(local, range);
        }
    }

    /// `export { … as default }` — the error `ExportNamedDeclaration.js`
    /// raises for a component script. Type-only specifiers are removed
    /// before analysis.
    fn check_default_export_specifiers(
        &mut self,
        specifiers: &[oxc_ast::ast::ExportSpecifier<'_>],
        span: oxc_span::Span,
    ) {
        use oxc_ast::ast::ModuleExportName;
        if self.hooks.is_none() {
            return;
        }
        let exports_default = specifiers.iter().any(|s| {
            !s.export_kind.is_type()
                && match &s.exported {
                    ModuleExportName::IdentifierName(id) => id.name == "default",
                    ModuleExportName::IdentifierReference(id) => id.name == "default",
                    ModuleExportName::StringLiteral(l) => l.value == "default",
                }
        });
        if exports_default {
            let range = self.abs(span.start, span.end);
            self.push_error(
                Code::module_illegal_default_export,
                crate::messages::module_illegal_default_export(),
                range,
            );
        }
    }

    fn visit_labeled(&mut self, lbl: &LabeledStatement<'_>, at_program_top: bool) {
        // For `reactive_declaration_module_script_dependency` we need to
        // know the reference sits inside a `$:` block at the top
        // level of the instance script.
        if let Some(h) = self.hooks {
            let range = self.abs(lbl.span.start, lbl.span.end);
            h.labeled_statement(
                &mut self.tree.script_rule_events,
                &self.ignore_frames,
                at_program_top,
                lbl,
                range,
            );
        }
        let is_top_level_reactive = lbl.label.name == "$" && self.is_instance && at_program_top;
        if is_top_level_reactive {
            // `$: x = …` / `$: ({ a, b } = obj)` — every identifier the
            // assignment writes (member targets excluded) may be an
            // implicit declaration; `$`-prefixed names never are.
            let mut body_expr = match &lbl.body {
                Statement::ExpressionStatement(es) => Some(&es.expression),
                _ => None,
            };
            // acorn drops the parens of `$: ({ a } = obj)`; oxc keeps them.
            while let Some(Expression::ParenthesizedExpression(p)) = body_expr {
                body_expr = Some(&p.expression);
            }
            if let Some(Expression::AssignmentExpression(a)) = body_expr {
                let mut idents = Vec::new();
                assignment_target_identifiers(&a.left, &mut idents);
                for (name, start, end) in idents {
                    if !name.starts_with('$') {
                        let range = self.abs(start, end);
                        self.tree
                            .implicit_reactive_decls
                            .push((SmolStr::from(name), range));
                    }
                }
            }
            // The compiler gives each top-level `$:` statement its own
            // non-porous scope, one function level deeper.
            let prev = std::mem::replace(&mut self.in_reactive_statement, true);
            let parent_scope = self.cur_scope();
            let scope = self.tree.new_scope(Some(parent_scope));
            self.tree.reactive_statements.push(ReactiveStatement {
                range: self.abs(lbl.span.start, lbl.span.end),
                scope,
                assignments: Vec::new(),
            });
            let prev_statement = self
                .reactive_statement
                .replace(self.tree.reactive_statements.len() - 1);
            let events_before = self.tree.script_rule_events.len();
            self.function_depth += 1;
            self.with_scope(scope, |w| w.visit_stmt(&lbl.body));
            self.function_depth -= 1;
            self.in_reactive_statement = prev;
            self.reactive_statement = prev_statement;
            // `LabeledStatement.js` walks a reactive statement's body
            // twice (once to collect its dependencies, then again), so
            // every script warning inside it is reported twice.
            let repeated = self.tree.script_rule_events[events_before..].to_vec();
            self.tree.script_rule_events.extend(repeated);
        } else {
            self.visit_stmt(&lbl.body);
        }
    }

    fn visit_class_decl(&mut self, cls: &Class<'_>) {
        // `declare class` is removed before analysis.
        if cls.declare {
            return;
        }
        if let Some(h) = self.hooks {
            let range = self.abs(cls.span.start, cls.span.end);
            h.class_declaration(
                &mut self.tree.script_rule_events,
                &self.ignore_frames,
                self.function_depth,
                range,
            );
        }
        if let Some(id) = &cls.id {
            self.declare_with_ignores(
                self.cur_scope(),
                SmolStr::from(id.name.as_str()),
                self.abs(id.span.start, id.span.end),
                BindingKind::Normal,
                DeclarationKind::Let,
                InitialKind::ClassDecl,
            );
        }
        self.visit_class_common(cls);
    }

    /// The parts shared by class declarations and class expressions:
    /// the super-class expression (a plain reference position) and
    /// the body. Upstream has NO `ClassExpression` visitor, so a
    /// named class expression's id is NOT declared anywhere — its
    /// body references resolve outward (verified against the
    /// compiler) — which is why this helper never touches `cls.id`.
    fn visit_class_common(&mut self, cls: &Class<'_>) {
        // The superclass expression moved under a `heritage` grouping
        // that also carries its type arguments; only the expression
        // holds value references.
        if let Some(heritage) = &cls.heritage {
            self.visit_expr(&heritage.expression);
        }
        self.visit_class_body(&cls.body);
    }

    fn visit_class_body(&mut self, body: &ClassBody<'_>) {
        let fields = if self.hooks.is_some_and(|h| h.runes) {
            self.check_class_fields(body)
        } else {
            Vec::new()
        };
        let prev_fields = std::mem::replace(&mut self.state_fields, fields);
        self.visit_class_members(body);
        self.state_fields = prev_fields;
    }

    /// `ClassBody.js` (runes mode): collect the class's state fields
    /// — fields and constructor `this.x = …` assignments initialised
    /// by `$state` / `$state.raw` / `$derived` / `$derived.by` — and
    /// reject a name declared twice, as the compiler does before
    /// walking the members.
    fn check_class_fields(&mut self, body: &ClassBody<'_>) -> Vec<(SmolStr, u32, bool)> {
        use crate::messages as m;
        let mut state_fields: Vec<(SmolStr, u32, bool)> = Vec::new();
        let mut fields: Vec<(String, Vec<&'static str>)> = Vec::new();
        let mut errors: Vec<(Code, String, oxc_span::Span)> = Vec::new();
        let is_state_rune = |value: Option<&Expression<'_>>| {
            value.is_some_and(|v| match unwrap_ts_wrappers(v) {
                Expression::CallExpression(c) => rune_keypath(&c.callee).is_some_and(|(r, _)| {
                    matches!(
                        r.as_str(),
                        "$state" | "$state.raw" | "$derived" | "$derived.by"
                    )
                }),
                _ => false,
            })
        };
        // The compiler's `handle(node, key, value)`.
        let handle = |state_fields: &mut Vec<(SmolStr, u32, bool)>,
                      fields: &[(String, Vec<&'static str>)],
                      errors: &mut Vec<(Code, String, oxc_span::Span)>,
                      span: oxc_span::Span,
                      name: Option<String>,
                      value: Option<&Expression<'_>>,
                      assignment: bool| {
            let Some(name) = name else { return };
            if !is_state_rune(value) {
                return;
            }
            if state_fields.iter().any(|(n, _, _)| n == name.as_str()) {
                errors.push((
                    Code::state_field_duplicate,
                    m::state_field_duplicate(&name),
                    span,
                ));
            }
            if let Some((_, kinds)) = fields.iter().find(|(k, _)| *k == name)
                && kinds.as_slice() != ["prop"]
            {
                errors.push((
                    Code::duplicate_class_field,
                    m::duplicate_class_field(&name),
                    span,
                ));
            }
            state_fields.push((SmolStr::from(name), span.start, assignment));
        };
        let mut constructor: Option<&oxc_ast::ast::Function<'_>> = None;
        for member in &body.body {
            match member {
                ClassElement::PropertyDefinition(p) if !p.declare && !p.computed && !p.r#static => {
                    handle(
                        &mut state_fields,
                        &fields,
                        &mut errors,
                        p.span,
                        class_key_name(&p.key),
                        p.value.as_ref(),
                        false,
                    );
                    let key = class_key_name(&p.key).unwrap_or_default();
                    if fields.iter().any(|(k, _)| *k == key) {
                        errors.push((
                            Code::duplicate_class_field,
                            m::duplicate_class_field(&key),
                            p.span,
                        ));
                    } else {
                        let kind = if p.value.is_some() {
                            "assigned_prop"
                        } else {
                            "prop"
                        };
                        fields.push((key, vec![kind]));
                    }
                }
                ClassElement::MethodDefinition(md)
                    if md.value.body.is_some()
                        && md.r#type
                            != oxc_ast::ast::MethodDefinitionType::TSAbstractMethodDefinition =>
                {
                    use oxc_ast::ast::MethodDefinitionKind as K;
                    if md.kind == K::Constructor {
                        constructor = Some(&md.value);
                        continue;
                    }
                    if md.computed {
                        continue;
                    }
                    let kind = match md.kind {
                        K::Get => "get",
                        K::Set => "set",
                        _ => "method",
                    };
                    let key = format!(
                        "{}{}",
                        if md.r#static { "@" } else { "" },
                        class_key_name(&md.key).unwrap_or_default()
                    );
                    let Some((_, existing)) = fields.iter_mut().find(|(k, _)| *k == key) else {
                        fields.push((key, vec![kind]));
                        continue;
                    };
                    if existing.contains(&kind)
                        || existing.contains(&"prop")
                        || existing.contains(&"assigned_prop")
                    {
                        errors.push((
                            Code::duplicate_class_field,
                            m::duplicate_class_field(&key),
                            md.span,
                        ));
                    }
                    let pairs = matches!(
                        (kind, existing.as_slice()),
                        ("get", ["set"]) | ("set", ["get"])
                    );
                    if pairs || kind == "method" {
                        existing.push(kind);
                        continue;
                    }
                    errors.push((
                        Code::duplicate_class_field,
                        m::duplicate_class_field(&key),
                        md.span,
                    ));
                }
                _ => {}
            }
        }
        if let Some(body) = constructor.and_then(|f| f.body.as_ref()) {
            for statement in &body.statements {
                let Statement::ExpressionStatement(es) = statement else {
                    continue;
                };
                let Expression::AssignmentExpression(a) = unwrap_ts_wrappers(&es.expression) else {
                    continue;
                };
                if !is_this_field_target(&a.left) {
                    continue;
                }
                handle(
                    &mut state_fields,
                    &fields,
                    &mut errors,
                    a.span,
                    this_field_name(&a.left),
                    Some(&a.right),
                    true,
                );
            }
        }
        if let Some((code, message, span)) = errors.into_iter().next() {
            let range = self.abs(span.start, span.end);
            self.push_error(code, message, range);
        }
        state_fields
    }

    fn visit_class_members(&mut self, body: &ClassBody<'_>) {
        for m in &body.body {
            // `declare` fields and abstract methods are removed before
            // analysis.
            let removed = match m {
                ClassElement::PropertyDefinition(p) => p.declare,
                ClassElement::MethodDefinition(md) => {
                    md.r#type == oxc_ast::ast::MethodDefinitionType::TSAbstractMethodDefinition
                }
                _ => false,
            };
            if removed {
                continue;
            }
            let pushed = self.push_leading_ignores(Some(m.span().start));
            match m {
                ClassElement::MethodDefinition(md) => {
                    self.visit_key(&md.key, md.computed);
                    let constructor = md.kind == oxc_ast::ast::MethodDefinitionKind::Constructor;
                    self.with_function(|w| {
                        if let Some(body) = &md.value.body {
                            for p in &md.value.params.items {
                                w.declare_pattern(&p.pattern, DeclarationKind::Param);
                            }
                            if let Some(rest) = &md.value.params.rest {
                                w.declare_pattern(&rest.rest.argument, DeclarationKind::RestParam);
                            }
                            if constructor {
                                w.visit_constructor_body(body);
                            } else {
                                w.visit_function_body(body, md.value.generator, true);
                            }
                        }
                    });
                }
                ClassElement::PropertyDefinition(p) => {
                    // `PropertyDefinition.js`: a field with a value may
                    // not precede the constructor assignment declaring
                    // a state field of the same name.
                    if p.value.is_some()
                        && let Some(name) = class_key_name(&p.key)
                        && let Some((_, start, _)) = self
                            .state_fields
                            .iter()
                            .find(|(n, _, _)| n.as_str() == name)
                        && *start != p.span.start
                        && p.span.start < *start
                    {
                        let range = self.abs(p.span.start, p.span.end);
                        self.push_error(
                            Code::state_field_invalid_assignment,
                            crate::messages::state_field_invalid_assignment(),
                            range,
                        );
                    }
                    self.visit_key(&p.key, p.computed);
                    if let Some(v) = &p.value {
                        // A rune call may initialise an instance field
                        // with a plain key.
                        let call = (!p.r#static && !p.computed)
                            .then(|| direct_call_span(v))
                            .flatten();
                        let prev = std::mem::replace(&mut self.field_init_call, call);
                        self.visit_expr(v);
                        self.field_init_call = prev;
                    }
                }
                ClassElement::AccessorProperty(p) => {
                    self.visit_key(&p.key, p.computed);
                    if let Some(v) = &p.value {
                        self.visit_expr(v);
                    }
                }
                ClassElement::StaticBlock(sb) => {
                    // Upstream `scope.js` has no StaticBlock visitor:
                    // no new scope, no function-depth bump — the
                    // statements walk in the enclosing scope, so a
                    // `$state` read inside fires
                    // `state_referenced_locally` (verified against
                    // the compiler).
                    for s in &sb.body {
                        self.visit_stmt(s);
                    }
                }
                ClassElement::TSIndexSignature(_) => {}
            }
            if pushed {
                self.ignore_frames.pop();
            }
        }
    }

    fn visit_var_decl(&mut self, vd: &VariableDeclaration<'_>) {
        // `declare const x: T` is removed before analysis.
        if vd.declare {
            return;
        }
        let decl_kind = match vd.kind {
            oxc_ast::ast::VariableDeclarationKind::Var => DeclarationKind::Var,
            oxc_ast::ast::VariableDeclarationKind::Let => DeclarationKind::Let,
            oxc_ast::ast::VariableDeclarationKind::Const => DeclarationKind::Const,
            oxc_ast::ast::VariableDeclarationKind::Using => DeclarationKind::Using,
            oxc_ast::ast::VariableDeclarationKind::AwaitUsing => DeclarationKind::AwaitUsing,
        };
        for declarator in &vd.declarations {
            self.visit_declarator(declarator, decl_kind);
        }
    }

    fn visit_declarator(&mut self, d: &VariableDeclarator<'_>, decl_kind: DeclarationKind) {
        // A comment between the declaration keyword and the pattern
        // (`const /* svelte-ignore … */ x = …`) leads the declarator
        // node, whose span starts at the pattern.
        let pushed = self.push_leading_ignores(Some(d.span.start));
        self.node_spans.push((d.span.start, d.span.end));
        self.visit_declarator_inner(d, decl_kind);
        self.node_spans.pop();
        if pushed {
            self.ignore_frames.pop();
        }
    }

    fn visit_declarator_inner(&mut self, d: &VariableDeclarator<'_>, decl_kind: DeclarationKind) {
        // Detect rune call on init. Must happen BEFORE we walk the
        // id/init so the bindings get the correct kind. Upstream's
        // `remove_typescript_nodes` pass strips `as`/`satisfies`/
        // `!`/`<T>(…)` expression wrappers before the analyze walk,
        // so we have to unwrap them here to match.
        let rune = d.init.as_ref().and_then(|e| match unwrap_ts_wrappers(e) {
            Expression::CallExpression(c) => detect_rune_call_from_call(c),
            _ => None,
        });

        let (binding_kind, initial) = match rune {
            Some(RuneCall::State) => {
                let primitive = d
                    .init
                    .as_ref()
                    .map(state_rune_primitive_arg)
                    .unwrap_or(StateArg::Proxied);
                (
                    BindingKind::State,
                    InitialKind::RuneCall {
                        rune: RuneCall::State,
                        primitive_arg: primitive,
                    },
                )
            }
            Some(RuneCall::StateRaw) => {
                let primitive = d
                    .init
                    .as_ref()
                    .map(state_rune_primitive_arg)
                    .unwrap_or(StateArg::Proxied);
                (
                    BindingKind::RawState,
                    InitialKind::RuneCall {
                        rune: RuneCall::StateRaw,
                        primitive_arg: primitive,
                    },
                )
            }
            Some(RuneCall::Derived) => (
                BindingKind::Derived,
                InitialKind::RuneCall {
                    rune: RuneCall::Derived,
                    primitive_arg: StateArg::Proxied,
                },
            ),
            Some(RuneCall::DerivedBy) => (
                BindingKind::Derived,
                InitialKind::RuneCall {
                    rune: RuneCall::DerivedBy,
                    primitive_arg: StateArg::Proxied,
                },
            ),
            Some(RuneCall::Props) => (
                BindingKind::Prop,
                InitialKind::RuneCall {
                    rune: RuneCall::Props,
                    primitive_arg: StateArg::Proxied,
                },
            ),
            _ => match d.init.as_ref() {
                None => (BindingKind::Normal, InitialKind::None),
                Some(e) => (
                    BindingKind::Normal,
                    InitialKind::Expression {
                        primitive: is_primitive_expr(e),
                    },
                ),
            },
        };

        // Declare each identifier in the pattern. If it's a $props()
        // destructure, the rest element becomes RestProp, and
        // `$bindable(default)` fallbacks flip to BindableProp.
        let is_props = matches!(rune, Some(RuneCall::Props));
        let is_props_identifier = is_props && matches!(&d.id, BindingPattern::BindingIdentifier(_));

        // custom_element_props_identifier candidate. Upstream
        // `VariableDeclarator.js:72-83` fires on Identifier form
        // (`let props = $props()` → id span) or ObjectPattern with
        // a rest element (`let { ...props } = $props()` → the
        // RestElement span). Firing is gated downstream by the
        // presence of `<svelte:options customElement={…}>` and the
        // absence of an explicit `props` option on it.
        if is_props {
            let warn_range = match &d.id {
                BindingPattern::BindingIdentifier(id) => Some(self.abs(id.span.start, id.span.end)),
                BindingPattern::ObjectPattern(op) => {
                    op.rest.as_ref().map(|r| self.abs(r.span.start, r.span.end))
                }
                _ => None,
            };
            if let Some(r) = warn_range {
                self.tree.custom_element_props_candidates.push(r);
                self.tree
                    .custom_element_props_ignored
                    .push(self.current_ignore_snapshot());
            }
        }

        self.declare_pattern_with(&d.id, decl_kind, binding_kind, &initial, is_props);

        if let Some(h) = self.hooks {
            self.check_module_import_conflict(&d.id);
            // `VariableDeclarator.js` re-validates every declared name
            // in runes mode, at any depth.
            if h.runes {
                for id in crate::scope_util::binding_idents_in_pattern(&d.id) {
                    if let Some((code, message)) = dollar_name_error(id.name.as_str()) {
                        let range = self.abs(id.span.start, id.span.end);
                        self.push_error(code, message, range);
                    }
                }
                if is_props {
                    self.check_props_pattern(d);
                }
            } else {
                self.check_legacy_rune_init(d);
            }
            // A `$bindable()` may only be the default of a property of
            // a `$props()` destructure.
            if is_props && let BindingPattern::ObjectPattern(op) = &d.id {
                for prop in &op.properties {
                    if let BindingPattern::AssignmentPattern(ap) = &prop.value
                        && let Expression::CallExpression(call) = unwrap_ts_wrappers(&ap.right)
                    {
                        self.bindable_positions
                            .push((call.span.start, call.span.end));
                    }
                }
            }
        }

        // `let { … } = $props()` bare identifier → RestProp (ambient-
        // style). Fix up the binding we just created.
        if is_props_identifier
            && let BindingPattern::BindingIdentifier(id) = &d.id
            && let Some(bid) = self
                .tree
                .scopes
                .get(self.cur_scope().0 as usize)
                .and_then(|s| s.declarations.get(id.name.as_str()).copied())
        {
            self.tree.bindings[bid.0 as usize].kind = BindingKind::RestProp;
        }

        // Upstream `VariableDeclarator.js:135-142`: for `$props()`
        // destructures, references inside default-value subpatterns
        // (e.g. `other_prop = prop`) are walked with function_depth+1
        // to prevent spurious `state_referenced_locally` fires on
        // prop-fallback references. We apply the bump by nudging
        // `rune_bump` for the pattern walk below.
        if is_props {
            self.rune_bump += 1;
            self.walk_pattern_defaults(&d.id);
            self.rune_bump -= 1;
        }

        // Walk the init expression so references inside get recorded.
        if let Some(init) = &d.init {
            // `$derived(...)` / `$inspect(...)` bump function_depth
            // for references inside the argument, mirroring upstream
            // `CallExpression.js:244-262`. Handled inside `visit_call`
            // below so we just continue the normal walk.
            let prev = std::mem::replace(&mut self.declarator_init_call, direct_call_span(init));
            let prev_ident = std::mem::replace(
                &mut self.declarator_binds_identifier,
                matches!(&d.id, BindingPattern::BindingIdentifier(_)),
            );
            self.visit_expr(init);
            self.declarator_init_call = prev;
            self.declarator_binds_identifier = prev_ident;
        }
    }

    /// Declare every identifier in a binding pattern. For `$props()`
    /// destructure: rest element → RestProp; `$bindable(x)` default →
    /// BindableProp.
    fn declare_pattern_with(
        &mut self,
        pat: &BindingPattern<'_>,
        decl_kind: DeclarationKind,
        kind: BindingKind,
        initial: &InitialKind,
        is_props: bool,
    ) {
        match pat {
            BindingPattern::BindingIdentifier(id) => {
                self.declare_with_ignores(
                    self.cur_scope(),
                    SmolStr::from(id.name.as_str()),
                    self.abs(id.span.start, id.span.end),
                    kind,
                    decl_kind,
                    initial.clone(),
                );
            }
            BindingPattern::ObjectPattern(op) => {
                self.declare_object_pattern(op, decl_kind, kind, initial, is_props);
            }
            BindingPattern::ArrayPattern(ap) => {
                self.declare_array_pattern(ap, decl_kind, kind, initial, is_props);
            }
            BindingPattern::AssignmentPattern(ap) => {
                // `let foo = default` — treat like the inner pattern.
                self.declare_pattern_with(&ap.left, decl_kind, kind, initial, is_props);
                // Walk the default-value expression so refs inside get
                // registered. For `$props()` destructures we defer
                // this walk to the caller so it can apply upstream's
                // `function_depth+1` bump (see
                // `VariableDeclarator.js:135-142` — prevents
                // `state_referenced_locally` false positives on
                // prop-fallback references). Non-props defaults walk
                // in place.
                if !is_props {
                    self.visit_expr(&ap.right);
                }
            }
        }
    }

    fn declare_object_pattern(
        &mut self,
        op: &ObjectPattern<'_>,
        decl_kind: DeclarationKind,
        kind: BindingKind,
        initial: &InitialKind,
        is_props: bool,
    ) {
        for prop in &op.properties {
            // For `$props()` destructure: upstream `VariableDeclarator.js`
            // rewires each binding's `initial` to the property default
            // (or None), NOT the outer `$props()` call — see
            // `2-analyze/visitors/VariableDeclarator.js:104-130`. So
            // `let { a } = $props()` leaves `a.initial = None`.
            let (child_kind, child_initial) = if is_props {
                if let Some(primitive) = detect_bindable_default(&prop.value) {
                    (
                        BindingKind::BindableProp,
                        InitialKind::RuneCall {
                            rune: RuneCall::Bindable,
                            primitive_arg: if primitive {
                                StateArg::Primitive
                            } else {
                                StateArg::Proxied
                            },
                        },
                    )
                } else {
                    // Unwrap an AssignmentPattern to see if there's a
                    // default expression.
                    let default = if let BindingPattern::AssignmentPattern(ap) = &prop.value {
                        InitialKind::Expression {
                            primitive: is_primitive_expr(&ap.right),
                        }
                    } else {
                        InitialKind::None
                    };
                    (BindingKind::Prop, default)
                }
            } else {
                (kind, initial.clone())
            };
            self.declare_pattern_with(&prop.value, decl_kind, child_kind, &child_initial, is_props);
        }
        if let Some(rest) = &op.rest {
            let child_kind = if is_props {
                BindingKind::RestProp
            } else {
                kind
            };
            // Upstream `VariableDeclarator.js` only walks the
            // ObjectPattern's `properties` list for the $props-rewire
            // step — rest-element bindings keep the `.initial` that
            // `scope.declare()` gave them, which is the $props()
            // CallExpression itself. Mirror that so
            // `store_rune_conflict`'s exception check (store_name ==
            // "props" && rune == $props → skip synthesis) fires
            // correctly.
            let child_initial = initial.clone();
            self.declare_pattern_with(&rest.argument, decl_kind, child_kind, &child_initial, false);
        }
    }

    fn declare_array_pattern(
        &mut self,
        ap: &ArrayPattern<'_>,
        decl_kind: DeclarationKind,
        kind: BindingKind,
        initial: &InitialKind,
        is_props: bool,
    ) {
        for p in ap.elements.iter().flatten() {
            self.declare_pattern_with(p, decl_kind, kind, initial, is_props);
        }
        if let Some(rest) = &ap.rest {
            self.declare_pattern_with(&rest.argument, decl_kind, kind, initial, is_props);
        }
    }

    fn declare_pattern(&mut self, pat: &BindingPattern<'_>, decl_kind: DeclarationKind) {
        self.declare_pattern_with(
            pat,
            decl_kind,
            BindingKind::Normal,
            &InitialKind::None,
            false,
        );
    }

    /// Walk the default-value expressions in an `AssignmentPattern`
    /// subtree. Callers drive this after bumping `rune_bump` so the
    /// references inside capture the elevated `function_depth_at_use`.
    fn walk_pattern_defaults(&mut self, pat: &BindingPattern<'_>) {
        match pat {
            BindingPattern::AssignmentPattern(ap) => {
                self.visit_expr(&ap.right);
                self.walk_pattern_defaults(&ap.left);
            }
            BindingPattern::ObjectPattern(op) => {
                for prop in &op.properties {
                    self.walk_pattern_defaults(&prop.value);
                }
                if let Some(rest) = &op.rest {
                    self.walk_pattern_defaults(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    self.walk_pattern_defaults(p);
                }
                if let Some(rest) = &ap.rest {
                    self.walk_pattern_defaults(&rest.argument);
                }
            }
            BindingPattern::BindingIdentifier(_) => {}
        }
    }

    /// Every expression gets a leading-ignore frame — upstream's
    /// analyze walk consults leadingComments at EVERY node, so a
    /// `// svelte-ignore` before a call argument, array element,
    /// object value, or initializer suppresses inside that subtree.
    /// The frame push is gated on the script having any ignore
    /// comment at all, so the common case adds one boolean check.
    fn visit_expr(&mut self, e: &Expression<'_>) {
        let pushed = self.push_leading_ignores(Some(e.span().start));
        let is_node = !matches!(
            e,
            Expression::ParenthesizedExpression(_)
                | Expression::TSAsExpression(_)
                | Expression::TSSatisfiesExpression(_)
                | Expression::TSNonNullExpression(_)
                | Expression::TSTypeAssertion(_)
                | Expression::TSInstantiationExpression(_)
        );
        if is_node {
            let span = e.span();
            self.node_spans.push((span.start, span.end));
        }
        self.visit_expr_inner(e);
        if is_node {
            self.node_spans.pop();
        }
        if pushed {
            self.ignore_frames.pop();
        }
    }

    fn visit_expr_inner(&mut self, e: &Expression<'_>) {
        if matches!(
            e,
            Expression::Identifier(_)
                | Expression::StaticMemberExpression(_)
                | Expression::ComputedMemberExpression(_)
        ) {
            self.check_rune_reference(e, self.call_of_callee(e));
        }
        match e {
            Expression::Identifier(id) => self.record_ref(id, RefParentKind::Read),
            Expression::ArrowFunctionExpression(arr) => {
                self.with_function(|w| {
                    for p in &arr.params.items {
                        w.declare_pattern(&p.pattern, DeclarationKind::Param);
                    }
                    if let Some(rest) = &arr.params.rest {
                        w.declare_pattern(&rest.rest.argument, DeclarationKind::RestParam);
                    }
                    match &arr.body {
                        oxc_ast::ast::ArrowFunctionBody::FunctionBody(body) => {
                            w.visit_function_body(body, false, false);
                        }
                        // A concise body holds an expression, which
                        // still references names the scope tree needs.
                        other => {
                            if let Some(expr) = other.as_expression() {
                                w.visit_expr(expr);
                            }
                        }
                    }
                });
            }
            Expression::FunctionExpression(f) => {
                self.with_function(|w| {
                    // A named function expression declares its own
                    // name inside its scope (upstream scope.js
                    // `FunctionExpression`: `scope.declare(node.id,
                    // 'normal', 'function')`) — body references to
                    // the name resolve to the function, not an outer
                    // binding of the same name.
                    if let Some(id) = &f.id {
                        w.declare_with_ignores(
                            w.cur_scope(),
                            SmolStr::from(id.name.as_str()),
                            w.abs(id.span.start, id.span.end),
                            BindingKind::Normal,
                            DeclarationKind::Function,
                            InitialKind::FunctionDecl,
                        );
                    }
                    for p in &f.params.items {
                        w.declare_pattern(&p.pattern, DeclarationKind::Param);
                    }
                    if let Some(rest) = &f.params.rest {
                        w.declare_pattern(&rest.rest.argument, DeclarationKind::RestParam);
                    }
                    if let Some(body) = &f.body {
                        w.visit_function_body(body, f.generator, true);
                    }
                });
            }
            Expression::CallExpression(c) => self.visit_call(c),
            Expression::NewExpression(n) => {
                if let Some(h) = self.hooks {
                    let range = self.abs(n.span.start, n.span.end);
                    h.new_expression(
                        &mut self.tree.script_rule_events,
                        &self.ignore_frames,
                        self.function_depth,
                        n,
                        range,
                    );
                }
                self.visit_expr(&n.callee);
                for a in &n.arguments {
                    self.visit_argument(a, false, false);
                }
            }
            Expression::ClassExpression(cls) => self.visit_class_common(cls),
            Expression::ImportExpression(ie) => {
                self.visit_expr(&ie.source);
                if let Some(opts) = &ie.options {
                    self.visit_expr(opts);
                }
            }
            Expression::PrivateInExpression(pie) => self.visit_expr(&pie.right),
            Expression::ObjectExpression(o) => self.visit_object(o),
            Expression::ArrayExpression(a) => {
                for el in &a.elements {
                    // `as_expression()` returns None for SpreadElement,
                    // so the spread's argument was silently skipped —
                    // any identifier inside `...(cond ? [a] : [])`
                    // wasn't being tracked, which made
                    // `export_let_unused` over-fire on props used only
                    // via spread-into-array.
                    use oxc_ast::ast::ArrayExpressionElement as AE;
                    match el {
                        AE::SpreadElement(s) => {
                            // Anchor leading ignores at the `...`,
                            // not the spread argument.
                            let pushed = self.push_leading_ignores(Some(s.span.start));
                            self.visit_expr(&s.argument);
                            if pushed {
                                self.ignore_frames.pop();
                            }
                        }
                        AE::Elision(_) => {}
                        other => {
                            if let Some(e) = other.as_expression() {
                                self.visit_expr(e);
                            }
                        }
                    }
                }
            }
            Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
            | Expression::PrivateFieldExpression(_) => {
                self.visit_member_expr(e);
            }
            Expression::BinaryExpression(b) => {
                self.visit_expr(&b.left);
                self.visit_expr(&b.right);
            }
            Expression::LogicalExpression(l) => {
                self.visit_expr(&l.left);
                self.visit_expr(&l.right);
            }
            Expression::ConditionalExpression(c) => {
                self.visit_expr(&c.test);
                self.visit_expr(&c.consequent);
                self.visit_expr(&c.alternate);
            }
            Expression::UnaryExpression(u) => self.visit_expr(&u.argument),
            Expression::AssignmentExpression(a) => self.visit_assignment(a),
            Expression::UpdateExpression(u) => self.visit_update(u),
            Expression::SequenceExpression(s) => {
                for e in &s.expressions {
                    self.visit_expr(e);
                }
            }
            Expression::ParenthesizedExpression(p) => {
                // `(/* svelte-ignore CODE */ expr)` — the recursive
                // visit_expr wrapper picks up comments leading the
                // inner expression.
                self.visit_expr(&p.expression);
            }
            Expression::StringLiteral(lit) => self.string_literal_hook(lit),
            Expression::TemplateLiteral(t) => {
                for e in &t.expressions {
                    self.visit_expr(e);
                }
                if let Some(h) = self.hooks {
                    h.template_literal(
                        &mut self.tree.script_rule_events,
                        &self.ignore_frames,
                        self.tree.bidi_warned.as_ref(),
                        t,
                        self.base_offset,
                    );
                }
            }
            Expression::TaggedTemplateExpression(t) => {
                self.visit_expr(&t.tag);
                for e in &t.quasi.expressions {
                    self.visit_expr(e);
                }
                // The template of a tagged template is an ordinary
                // template literal to the compiler's visitors.
                if let Some(h) = self.hooks {
                    h.template_literal(
                        &mut self.tree.script_rule_events,
                        &self.ignore_frames,
                        self.tree.bidi_warned.as_ref(),
                        &t.quasi,
                        self.base_offset,
                    );
                }
            }
            Expression::AwaitExpression(a) => {
                // Upstream flips runes mode on any await whose
                // ancestor path contains no function (scope.js
                // AwaitExpression) — `in_function_closure` tracks
                // exactly the three excluded node types.
                if self.counts_await && !self.in_function_closure {
                    self.tree.has_await = true;
                }
                // `AwaitExpression.js`: a top-level instance `await`
                // (function depth 1, which `$:` bodies, `$inspect` and
                // `$props()` defaults raise) or one inside a template
                // expression / `$derived(…)` suspends, which needs the
                // `experimental.async` option and runes mode.
                let top_level = self.is_instance && self.function_depth == 1 && self.rune_bump == 0;
                if self.hooks.is_some() && (top_level || self.in_reactive_expression) {
                    let range = self.abs(a.span.start, a.span.end);
                    self.tree
                        .script_rule_events
                        .push(ScriptRuleEvent::SuspendingAwait { range });
                }
                self.visit_expr(&a.argument)
            }
            Expression::YieldExpression(y) => {
                if let Some(arg) = &y.argument {
                    self.visit_expr(arg);
                }
            }
            Expression::TSAsExpression(t) => self.visit_expr(&t.expression),
            Expression::TSSatisfiesExpression(t) => self.visit_expr(&t.expression),
            Expression::TSNonNullExpression(t) => self.visit_expr(&t.expression),
            Expression::TSTypeAssertion(t) => self.visit_expr(&t.expression),
            Expression::TSInstantiationExpression(t) => self.visit_expr(&t.expression),
            Expression::ChainExpression(ch) => self.visit_chain_element(&ch.expression),
            _ => {}
        }
    }

    fn visit_member_expr(&mut self, e: &Expression<'_>) {
        match e {
            Expression::StaticMemberExpression(m) => {
                // `MemberExpression.js`: a `$$` name read off a
                // `$props()` rest binding.
                if self.hooks.is_some()
                    && let Expression::Identifier(object) = &m.object
                    && m.property.name.starts_with("$$")
                {
                    let range = self.abs(m.property.span.start, m.property.span.end);
                    self.push_gated_error(
                        ErrorGate::RestProp {
                            object: SmolStr::from(object.name.as_str()),
                            scope: self.cur_scope(),
                        },
                        Code::props_illegal_name,
                        crate::messages::props_illegal_name(),
                        range,
                    );
                }
                self.visit_member_object(&m.object)
            }
            Expression::ComputedMemberExpression(m) => {
                self.visit_member_object(&m.object);
                self.visit_expr(&m.expression);
            }
            Expression::PrivateFieldExpression(m) => self.visit_member_object(&m.object),
            _ => {}
        }
    }

    /// Visit the object of a MemberExpression, tagging direct
    /// identifier reads with `RefParentKind::MemberObject`. Non-
    /// identifier expressions (nested `(x.y).z`, calls, etc.) fall
    /// through to the regular visitor.
    fn visit_member_object(&mut self, e: &Expression<'_>) {
        if let Expression::Identifier(id) = e {
            self.record_ref(id, RefParentKind::MemberObject);
        } else {
            self.visit_expr(e);
        }
    }

    fn visit_chain_element(&mut self, el: &ChainElement<'_>) {
        match el {
            ChainElement::CallExpression(c) => self.visit_call(c),
            ChainElement::StaticMemberExpression(m) => self.visit_member_object(&m.object),
            ChainElement::ComputedMemberExpression(m) => {
                self.visit_member_object(&m.object);
                self.visit_expr(&m.expression);
            }
            ChainElement::PrivateFieldExpression(m) => self.visit_member_object(&m.object),
            _ => {}
        }
    }

    fn visit_object(&mut self, o: &ObjectExpression<'_>) {
        for p in &o.properties {
            // Leading ignores anchor at the PROPERTY span (the key,
            // or the `...` of a spread) — a comment before `open:`
            // must suppress inside the value, whose own span starts
            // after the key and colon.
            let pushed = self.push_leading_ignores(Some(p.span().start));
            match p {
                ObjectPropertyKind::ObjectProperty(op) => {
                    self.visit_key(&op.key, op.computed);
                    self.visit_expr(&op.value);
                }
                // `{ ...rest }` — walk the spread argument so
                // identifiers inside (`adminUser`, etc.) register as
                // references. Previously the match-guard only
                // matched ObjectProperty, silently dropping spread
                // properties and under-counting references.
                ObjectPropertyKind::SpreadProperty(s) => {
                    self.visit_expr(&s.argument);
                }
            }
            if pushed {
                self.ignore_frames.pop();
            }
        }
    }

    fn visit_call(&mut self, c: &CallExpression<'_>) {
        // If this is a $derived(...) / $inspect(...) call, bump the
        // analyze-phase function_depth for its arguments.
        let rune = detect_rune_call_from_call(c);
        let bump = matches!(rune, Some(RuneCall::Derived) | Some(RuneCall::Inspect));
        // Track `nested_in_state_call` for refs inside arg subtrees —
        // used by state_referenced_locally's message discriminator.
        let push_state = matches!(rune, Some(RuneCall::State) | Some(RuneCall::StateRaw));
        if self.hooks.is_some() {
            self.check_rune_call(c);
        }
        // Callee — flag the identifier (if any) as a child of the
        // CallExpression for `store_rune_conflict`'s sake; arguments
        // are flagged in `visit_argument`.
        let callee = unwrap_ts_wrappers(&c.callee).span();
        let prev_callee = self
            .callee_span
            .replace(((callee.start, callee.end), (c.span.start, c.span.end)));
        self.visit_callee(&c.callee);
        self.callee_span = prev_callee;
        if bump {
            self.rune_bump += 1;
        }
        let prev_reactive = self.in_reactive_expression;
        if rune == Some(RuneCall::Derived) {
            self.in_reactive_expression = true;
        }
        for a in &c.arguments {
            self.visit_argument(a, push_state, true);
        }
        self.in_reactive_expression = prev_reactive;
        if bump {
            self.rune_bump -= 1;
        }
    }

    /// The errors `CallExpression.js` raises for a rune call — the
    /// argument checks and where each rune may stand — in the order the
    /// compiler raises them. Every one is gated on the rune name not
    /// resolving to a binding, which is how the compiler recognises a
    /// rune call.
    fn check_rune_call(&mut self, c: &CallExpression<'_>) {
        use crate::messages as m;
        let Some((rune, root)) = rune_keypath(&c.callee) else {
            return;
        };
        let range = self.abs(c.span.start, c.span.end);
        let span = Some((c.span.start, c.span.end));
        let args = c.arguments.len();
        let at_instance_top = self.is_instance && self.cur_scope() == self.scope_stack[0];
        let mut errors: Vec<(Code, String)> = Vec::new();
        if rune != "$inspect"
            && c.arguments
                .iter()
                .any(|a| matches!(a, oxc_ast::ast::Argument::SpreadElement(_)))
        {
            errors.push((Code::rune_invalid_spread, m::rune_invalid_spread(&rune)));
        }
        let exactly_one = |errors: &mut Vec<(Code, String)>| {
            if args != 1 {
                errors.push((
                    Code::rune_invalid_arguments_length,
                    m::rune_invalid_arguments_length(&rune, "exactly one argument"),
                ));
            }
        };
        match rune.as_str() {
            "$bindable" => {
                if args > 1 {
                    errors.push((
                        Code::rune_invalid_arguments_length,
                        m::rune_invalid_arguments_length(&rune, "zero or one arguments"),
                    ));
                }
                if !self
                    .bindable_positions
                    .contains(&(c.span.start, c.span.end))
                {
                    errors.push((
                        Code::bindable_invalid_location,
                        m::bindable_invalid_location(),
                    ));
                }
            }
            "$host" => {
                if args > 0 {
                    errors.push((
                        Code::rune_invalid_arguments,
                        m::rune_invalid_arguments(&rune),
                    ));
                } else {
                    // Whether the component is a custom element is
                    // known only once the template is read.
                    self.push_rune_errors(&root, std::mem::take(&mut errors), range);
                    let host_gate = ErrorGate::Host {
                        scope: self.cur_scope(),
                        module: self.write_origin() == WriteOrigin::Module,
                    };
                    self.push_gated_error(
                        host_gate,
                        Code::host_invalid_placement,
                        m::host_invalid_placement(),
                        range,
                    );
                    return;
                }
            }
            "$props" => {
                self.props_calls += 1;
                if self.props_calls > 1 {
                    errors.push((Code::props_duplicate, m::props_duplicate(&rune)));
                }
                if self.declarator_init_call != span || !at_instance_top {
                    errors.push((Code::props_invalid_placement, m::props_invalid_placement()));
                }
                if args > 0 {
                    errors.push((
                        Code::rune_invalid_arguments,
                        m::rune_invalid_arguments(&rune),
                    ));
                }
            }
            "$props.id" => {
                self.props_id_calls += 1;
                if self.props_id_calls > 1 {
                    errors.push((Code::props_duplicate, m::props_duplicate(&rune)));
                }
                if self.declarator_init_call != span
                    || !self.declarator_binds_identifier
                    || !at_instance_top
                {
                    errors.push((
                        Code::props_id_invalid_placement,
                        m::props_id_invalid_placement(),
                    ));
                }
                if args > 0 {
                    errors.push((
                        Code::rune_invalid_arguments,
                        m::rune_invalid_arguments(&rune),
                    ));
                }
            }
            "$state" | "$state.raw" | "$derived" | "$derived.by" => {
                let valid = self.declarator_init_call == span
                    || self.field_init_call == span
                    || self.constructor_assignment_call == span;
                if !valid {
                    errors.push((
                        Code::state_invalid_placement,
                        m::state_invalid_placement(&rune),
                    ));
                }
                if rune.starts_with("$derived") {
                    exactly_one(&mut errors);
                } else if args > 1 {
                    errors.push((
                        Code::rune_invalid_arguments_length,
                        m::rune_invalid_arguments_length(&rune, "zero or one arguments"),
                    ));
                }
            }
            "$effect" | "$effect.pre" => {
                if self.statement_call != span {
                    errors.push((
                        Code::effect_invalid_placement,
                        m::effect_invalid_placement(),
                    ));
                }
                exactly_one(&mut errors);
            }
            "$effect.tracking" => {
                if args != 0 {
                    errors.push((
                        Code::rune_invalid_arguments,
                        m::rune_invalid_arguments(&rune),
                    ));
                }
            }
            "$effect.root" | "$inspect().with" | "$state.eager" | "$state.snapshot" => {
                exactly_one(&mut errors);
            }
            "$inspect" => {
                if args < 1 {
                    errors.push((
                        Code::rune_invalid_arguments_length,
                        m::rune_invalid_arguments_length(&rune, "one or more arguments"),
                    ));
                }
            }
            "$inspect.trace" => {
                if args > 1 {
                    errors.push((
                        Code::rune_invalid_arguments_length,
                        m::rune_invalid_arguments_length(&rune, "zero or one arguments"),
                    ));
                }
                match self.trace_slot {
                    Some((start, end, generator)) if Some((start, end)) == span => {
                        if generator {
                            errors.push((
                                Code::inspect_trace_generator,
                                m::inspect_trace_generator(),
                            ));
                        }
                    }
                    _ => errors.push((
                        Code::inspect_trace_invalid_placement,
                        m::inspect_trace_invalid_placement(),
                    )),
                }
            }
            _ => {}
        }
        self.push_rune_errors(&root, errors, range);
    }

    fn push_rune_errors(&mut self, root: &str, errors: Vec<(Code, String)>, range: Range) {
        let scope = self.cur_scope();
        for (code, message) in errors {
            self.push_gated_error(
                ErrorGate::Unshadowed {
                    name: SmolStr::from(root),
                    scope,
                },
                code,
                message,
                range,
            );
        }
    }

    /// The call whose callee `e` is, if any.
    fn call_of_callee(&self, e: &Expression<'_>) -> Option<(u32, u32)> {
        let span = e.span();
        self.callee_span
            .filter(|(callee, _)| *callee == (span.start, span.end))
            .map(|(_, call)| call)
    }

    /// `Identifier.js` in runes mode: a rune name may only be read as a
    /// call's callee, through a member chain that spells a rune
    /// (`$state.raw`). `e` is an identifier or member chain rooted at
    /// one; `parent_call` is the call whose callee `e` is.
    fn check_rune_reference(&mut self, e: &Expression<'_>, parent_call: Option<(u32, u32)>) {
        let parent_is_call = parent_call.is_some();
        use crate::messages as m;
        if !self.hooks.is_some_and(|h| h.runes) {
            return;
        }
        // The chain from the root identifier outwards.
        let mut chain: Vec<&Expression<'_>> = vec![e];
        let mut cur = e;
        loop {
            cur = match cur {
                Expression::StaticMemberExpression(mem) => &mem.object,
                Expression::ComputedMemberExpression(mem) => &mem.object,
                _ => break,
            };
            chain.push(cur);
        }
        chain.reverse();
        let Some(Expression::Identifier(root)) = chain.first() else {
            return;
        };
        let root_name = root.name.as_str();
        if !is_rune_name(root_name) {
            return;
        }
        // The node enclosing `e` (the compiler reports some errors on
        // it): the call when `e` is a callee, else the enclosing node
        // the walk recorded.
        let outer_parent = if parent_is_call {
            parent_call
        } else {
            let n = self.node_spans.len();
            // `node_spans` ends with `e` itself when it came through
            // `visit_expr`.
            let top = self.node_spans.last().copied();
            let espan = (e.span().start, e.span().end);
            if top == Some(espan) {
                n.checked_sub(2)
                    .and_then(|i| self.node_spans.get(i).copied())
            } else {
                top
            }
        };
        let mut name = root_name.to_string();
        let mut error: Option<(Code, String, (u32, u32))> = None;
        for (i, link) in chain.iter().enumerate().skip(1) {
            let link_span = (link.span().start, link.span().end);
            let parent = match chain.get(i + 1) {
                Some(p) => Some((p.span().start, p.span().end)),
                None => outer_parent,
            };
            let property = match link {
                Expression::StaticMemberExpression(mem) => mem.property.name.as_str(),
                Expression::ComputedMemberExpression(_) => {
                    error = Some((
                        Code::rune_invalid_computed_property,
                        m::rune_invalid_computed_property(),
                        link_span,
                    ));
                    break;
                }
                _ => break,
            };
            name.push('.');
            name.push_str(property);
            if !is_rune_name(&name) {
                let Some(parent) = parent else {
                    return;
                };
                let (code, message) = match name.as_str() {
                    "$effect.active" => (
                        Code::rune_renamed,
                        m::rune_renamed("$effect.active", "$effect.tracking"),
                    ),
                    "$state.frozen" => (
                        Code::rune_renamed,
                        m::rune_renamed("$state.frozen", "$state.raw"),
                    ),
                    "$state.is" => (Code::rune_removed, m::rune_removed("$state.is")),
                    _ => (Code::rune_invalid_name, m::rune_invalid_name(&name)),
                };
                error = Some((code, message, parent));
                break;
            }
        }
        if error.is_none() && !parent_is_call {
            let espan = e.span();
            error = Some((
                Code::rune_missing_parentheses,
                m::rune_missing_parentheses(),
                (espan.start, espan.end),
            ));
        }
        if let Some((code, message, (start, end))) = error {
            let range = self.abs(start, end);
            let scope = self.cur_scope();
            self.push_gated_error(
                ErrorGate::Unshadowed {
                    name: SmolStr::from(root_name),
                    scope,
                },
                code,
                message,
                range,
            );
        }
    }

    /// `ensure_no_module_import_conflict`: a top-level instance
    /// declaration may not reuse the name of a `<script module>`
    /// import.
    fn check_module_import_conflict(&mut self, id: &BindingPattern<'_>) {
        if !self.is_instance || self.cur_scope() != self.scope_stack[0] {
            return;
        }
        let Some(module_root) = self.tree.scopes[self.cur_scope().0 as usize].parent else {
            return;
        };
        let module_scope = &self.tree.scopes[module_root.0 as usize];
        let conflict = crate::scope_util::binding_idents_in_pattern(id)
            .iter()
            .any(|ident| {
                module_scope
                    .declarations
                    .get(ident.name.as_str())
                    .is_some_and(|b| {
                        self.tree.bindings[b.0 as usize].declaration_kind == DeclarationKind::Import
                    })
            });
        if conflict {
            let span = id.span();
            let range = self.abs(span.start, span.end);
            self.push_error(
                Code::declaration_duplicate_module_import,
                crate::messages::declaration_duplicate_module_import(),
                range,
            );
        }
    }

    /// The pattern a runes-mode `$props()` declarator may bind: an
    /// identifier, or an object destructure of plain, non-computed,
    /// non-`$$` properties.
    fn check_props_pattern(&mut self, d: &VariableDeclarator<'_>) {
        let scope = self.cur_scope();
        let gate = || ErrorGate::Unshadowed {
            name: SmolStr::new_static("$props"),
            scope,
        };
        match &d.id {
            BindingPattern::BindingIdentifier(_) => {}
            BindingPattern::ObjectPattern(op) => {
                for prop in &op.properties {
                    let range = self.abs(prop.span.start, prop.span.end);
                    if prop.computed {
                        self.push_gated_error(
                            gate(),
                            Code::props_invalid_pattern,
                            crate::messages::props_invalid_pattern(),
                            range,
                        );
                    }
                    if let PropertyKey::StaticIdentifier(key) = &prop.key
                        && key.name.starts_with("$$")
                    {
                        self.push_gated_error(
                            gate(),
                            Code::props_illegal_name,
                            crate::messages::props_illegal_name(),
                            range,
                        );
                    }
                    let value = match &prop.value {
                        BindingPattern::AssignmentPattern(ap) => &ap.left,
                        other => other,
                    };
                    if !matches!(value, BindingPattern::BindingIdentifier(_)) {
                        self.push_gated_error(
                            gate(),
                            Code::props_invalid_pattern,
                            crate::messages::props_invalid_pattern(),
                            range,
                        );
                    }
                }
            }
            _ => {
                let range = self.abs(d.span.start, d.span.end);
                self.push_gated_error(
                    gate(),
                    Code::props_invalid_identifier,
                    crate::messages::props_invalid_identifier(),
                    range,
                );
            }
        }
    }

    /// Outside runes mode a declarator may not be initialised by a
    /// `$state`, `$derived` or `$props` call (unless the name is a
    /// store subscription).
    fn check_legacy_rune_init(&mut self, d: &VariableDeclarator<'_>) {
        let Some(Expression::CallExpression(c)) = d.init.as_ref().map(unwrap_ts_wrappers) else {
            return;
        };
        let Expression::Identifier(callee) = &c.callee else {
            return;
        };
        let name = callee.name.as_str();
        if matches!(name, "$state" | "$derived" | "$props") {
            let range = self.abs(c.span.start, c.span.end);
            self.push_gated_error(
                ErrorGate::NotStoreSub {
                    name: SmolStr::from(name),
                    scope: self.cur_scope(),
                },
                Code::rune_invalid_usage,
                crate::messages::rune_invalid_usage(name),
                range,
            );
        }
    }

    fn push_gated_error(&mut self, gate: ErrorGate, code: Code, message: String, range: Range) {
        self.tree
            .script_rule_events
            .push(ScriptRuleEvent::GatedError {
                gate,
                code,
                message,
                range,
            });
    }

    fn push_error(&mut self, code: Code, message: String, range: Range) {
        self.tree.script_rule_events.push(ScriptRuleEvent::Error {
            code,
            message,
            range,
        });
    }

    /// One call/new argument. `as_expression()` is None for
    /// `Argument::SpreadElement`, so spreads need their own arm —
    /// `f(...props)` must record the references inside the spread
    /// argument (verified: upstream counts them, and a `$state` read
    /// inside `f(...[count])` fires `state_referenced_locally`).
    fn visit_argument(
        &mut self,
        a: &oxc_ast::ast::Argument<'_>,
        in_state_call: bool,
        of_call: bool,
    ) {
        if let oxc_ast::ast::Argument::SpreadElement(s) = a {
            // Anchor leading ignores at the `...`, mirroring the
            // array-spread path.
            let pushed = self.push_leading_ignores(Some(s.span.start));
            // A spread's inner identifier is never the DIRECT rune
            // argument, so upstream's ancestor walk always sees it as
            // nested — flag accordingly inside `$state(…)` calls.
            let saved = self.in_state_arg_nested;
            if in_state_call {
                self.in_state_arg_nested = true;
            }
            self.visit_expr(&s.argument);
            self.in_state_arg_nested = saved;
            if pushed {
                self.ignore_frames.pop();
            }
        } else if let Some(e) = a.as_expression() {
            // A `$name` passed straight to a call has the call as its
            // parent node once parentheses and type wrappers are gone,
            // which is what `store_rune_conflict` asks about.
            let mut bare = e;
            loop {
                bare = match bare {
                    Expression::ParenthesizedExpression(p) => &p.expression,
                    Expression::TSAsExpression(x) => &x.expression,
                    Expression::TSSatisfiesExpression(x) => &x.expression,
                    Expression::TSNonNullExpression(x) => &x.expression,
                    Expression::TSTypeAssertion(x) => &x.expression,
                    _ => break,
                };
            }
            if of_call
                && let Expression::Identifier(id) = bare
                && id.name.starts_with('$')
            {
                let pushed = self.push_leading_ignores(Some(e.span().start));
                self.check_rune_reference(bare, None);
                self.record_ref_id_full(
                    id.name.as_str(),
                    id.span.start,
                    id.span.end,
                    RefParentKind::Read,
                    true,
                );
                if pushed {
                    self.ignore_frames.pop();
                }
                return;
            }
            if in_state_call {
                self.visit_arg_inside_state_call(e);
            } else {
                self.visit_expr(e);
            }
        }
    }

    fn visit_callee(&mut self, e: &Expression<'_>) {
        match e {
            Expression::Identifier(id) => {
                self.record_ref_id_full(
                    id.name.as_str(),
                    id.span.start,
                    id.span.end,
                    RefParentKind::Read,
                    true,
                );
            }
            _ => self.visit_expr(e),
        }
    }

    /// Walk an expression that is a direct argument of $state(...) /
    /// $state.raw(...). References inside it are tagged
    /// `nested_in_state_call = true` ONLY when they are below a
    /// further expression level — direct-arg identifiers mirror
    /// upstream's ancestor-walk bug where path[i+1] == undefined and
    /// "derived" is missed. See `notes/lint.md §4.4`.
    fn visit_arg_inside_state_call(&mut self, e: &Expression<'_>) {
        // The direct-identifier branch bypasses the visit_expr
        // wrapper, so pick up leading ignores here.
        let pushed = self.push_leading_ignores(Some(e.span().start));
        match e {
            // Direct identifier / member at top level: NOT flagged
            // (mirrors upstream bug).
            Expression::Identifier(id) => {
                self.check_rune_reference(e, None);
                self.record_ref(id, RefParentKind::Read);
            }
            // Nested — walk with the flag ON.
            _ => {
                let saved = std::mem::replace(&mut self.in_state_arg_nested, true);
                self.visit_expr(e);
                self.in_state_arg_nested = saved;
            }
        }
        if pushed {
            self.ignore_frames.pop();
        }
    }

    /// What an assignment inside a `$:` statement tells the compiler's
    /// reactive-statement ordering: the identifiers it assigns (not
    /// members), and — for a plain `=` — the target identifier or
    /// member root, whose reference is then no dependency.
    fn note_reactive_assignment(&mut self, index: usize, a: &AssignmentExpression<'_>) {
        let mut names = Vec::new();
        assignment_target_identifiers(&a.left, &mut names);
        let scope = self.cur_scope();
        for (name, _, _) in names {
            self.tree.reactive_statements[index]
                .assignments
                .push((SmolStr::from(name), scope));
        }
        if a.operator == oxc_syntax::operator::AssignmentOperator::Assign {
            let target = match &a.left {
                AssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.span.start),
                other => other
                    .as_member_expression()
                    .and_then(|m| member_root_identifier(m.object()))
                    .map(|id| id.span.start),
            };
            if let Some(start) = target {
                self.tree
                    .reactive_assignment_targets
                    .push(start + self.base_offset);
            }
        }
    }

    /// The state-field half of the compiler's `validate_assignment`: in
    /// a constructor, a write to a state field may not precede the
    /// assignment declaring it.
    fn check_state_field_write(&mut self, name: Option<String>, span: oxc_span::Span) {
        if !self.in_constructor_body {
            return;
        }
        let Some(name) = name else { return };
        let Some(&(_, declared_at, by_assignment)) = self
            .state_fields
            .iter()
            .find(|(n, _, _)| n.as_str() == name)
        else {
            return;
        };
        if by_assignment && declared_at != span.start && span.start < declared_at {
            let range = self.abs(span.start, span.end);
            self.push_error(
                Code::state_field_invalid_assignment,
                crate::messages::state_field_invalid_assignment(),
                range,
            );
        }
    }

    fn visit_assignment(&mut self, a: &AssignmentExpression<'_>) {
        if !self.state_fields.is_empty() && is_this_member_target(&a.left) {
            self.check_state_field_write(this_field_name(&a.left), a.span);
        }
        if let Some(index) = self.reactive_statement {
            self.note_reactive_assignment(index, a);
        }
        let mut targets = Vec::new();
        checked_write_targets(&a.left, &mut targets);
        let bare = matches!(a.left, AssignmentTarget::AssignmentTargetIdentifier(_));
        self.push_write(targets, bare, a.span);
        // Record the target.
        self.visit_assignment_target(&a.left);
        self.visit_expr(&a.right);
    }

    fn visit_assignment_target(&mut self, t: &AssignmentTarget<'_>) {
        self.visit_assignment_target_as(t, RefParentKind::AssignmentLeft);
    }

    /// `id_kind` is the parent kind recorded when `t` itself is a bare
    /// identifier. Only the direct `left` of an assignment is a
    /// write-position reference to the compiler (`parent.left ===
    /// node`); identifiers inside a destructuring target sit under an
    /// ArrayPattern / Property / RestElement / AssignmentPattern and
    /// count as reads for `state_referenced_locally`, so nested
    /// targets recurse with `Read`. The update bookkeeping is the same
    /// for both.
    fn visit_assignment_target_as(&mut self, t: &AssignmentTarget<'_>, id_kind: RefParentKind) {
        match t {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                // foo = …
                self.record_ref_id(id.name.as_str(), id.span.start, id.span.end, id_kind);
                self.tree.pending_updates.push(PendingUpdate {
                    scope: self.cur_scope(),
                    name: SmolStr::from(id.name.as_str()),
                    range: self.abs(id.span.start, id.span.end),
                    is_reassign: true,
                });
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                // foo.bar = …  → mutation of base
                if let Some(base) = base_identifier(&m.object) {
                    self.record_ref_id(
                        base.0,
                        base.1,
                        base.2,
                        RefParentKind::MemberObjectOfAssignment,
                    );
                    self.tree.pending_updates.push(PendingUpdate {
                        scope: self.cur_scope(),
                        name: SmolStr::from(base.0),
                        range: self.abs(base.1, base.2),
                        is_reassign: false,
                    });
                }
                self.visit_expr(&m.object);
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                if let Some(base) = base_identifier(&m.object) {
                    self.record_ref_id(
                        base.0,
                        base.1,
                        base.2,
                        RefParentKind::MemberObjectOfAssignment,
                    );
                    self.tree.pending_updates.push(PendingUpdate {
                        scope: self.cur_scope(),
                        name: SmolStr::from(base.0),
                        range: self.abs(base.1, base.2),
                        is_reassign: false,
                    });
                }
                self.visit_expr(&m.object);
                self.visit_expr(&m.expression);
            }
            AssignmentTarget::ArrayAssignmentTarget(arr) => {
                // `[el] = pair` — upstream drains updates through
                // `unwrap_pattern`, so every leaf target counts as a
                // reassignment (identifier form) or mutation (member
                // form). Recursing through `visit_assignment_target`
                // reproduces that per leaf.
                for el in arr.elements.iter().flatten() {
                    self.visit_assignment_target_maybe_default(el);
                }
                if let Some(rest) = &arr.rest {
                    self.visit_assignment_target_as(&rest.target, RefParentKind::Read);
                }
            }
            AssignmentTarget::ObjectAssignmentTarget(obj) => {
                for p in &obj.properties {
                    use oxc_ast::ast::AssignmentTargetProperty as ATP;
                    match p {
                        ATP::AssignmentTargetPropertyIdentifier(pi) => {
                            // Shorthand `({ el } = obj)` — the binding
                            // IS the target identifier.
                            self.record_ref_id(
                                pi.binding.name.as_str(),
                                pi.binding.span.start,
                                pi.binding.span.end,
                                RefParentKind::Read,
                            );
                            self.tree.pending_updates.push(PendingUpdate {
                                scope: self.cur_scope(),
                                name: SmolStr::from(pi.binding.name.as_str()),
                                range: self.abs(pi.binding.span.start, pi.binding.span.end),
                                is_reassign: true,
                            });
                            if let Some(init) = &pi.init {
                                self.visit_expr(init);
                            }
                        }
                        ATP::AssignmentTargetPropertyProperty(pp) => {
                            self.visit_key(&pp.name, pp.computed);
                            self.visit_assignment_target_maybe_default(&pp.binding);
                        }
                    }
                }
                if let Some(rest) = &obj.rest {
                    self.visit_assignment_target_as(&rest.target, RefParentKind::Read);
                }
            }
            _ => {}
        }
    }

    fn visit_assignment_target_maybe_default(
        &mut self,
        t: &oxc_ast::ast::AssignmentTargetMaybeDefault<'_>,
    ) {
        use oxc_ast::ast::AssignmentTargetMaybeDefault as ATMD;
        match t {
            ATMD::AssignmentTargetWithDefault(d) => {
                self.visit_assignment_target_as(&d.binding, RefParentKind::Read);
                self.visit_expr(&d.init);
            }
            other => {
                if let Some(target) = other.as_assignment_target() {
                    self.visit_assignment_target_as(target, RefParentKind::Read);
                }
            }
        }
    }

    /// Shared walk for `for (… of …)` / `for (… in …)`. Upstream
    /// wraps the whole statement in a porous block scope
    /// (`create_block_scope`), so a declaration left shadows outer
    /// bindings; an identifier/pattern left records plain READ
    /// references — updates come only from AssignmentExpression /
    /// UpdateExpression, so `for (x of xs)` is NOT a reassignment
    /// (verified against the compiler).
    fn visit_for_in_of(
        &mut self,
        left: &oxc_ast::ast::ForStatementLeft<'_>,
        right: &Expression<'_>,
        body: &Statement<'_>,
    ) {
        use oxc_ast::ast::ForStatementLeft as FSL;
        let s = self.tree.new_porous_scope(self.cur_scope());
        self.with_scope(s, |w| {
            match left {
                FSL::VariableDeclaration(v) => w.visit_var_decl(v),
                other => {
                    if let Some(target) = other.as_assignment_target() {
                        w.visit_target_reads(target);
                    }
                }
            }
            w.visit_expr(right);
            w.visit_stmt(body);
        });
    }

    /// Record READ references for every identifier leaf of an
    /// assignment-target pattern, without registering updates. Only
    /// the for-of/for-in left-hand side needs this weaker walk — see
    /// [`Self::visit_for_in_of`].
    fn visit_target_reads(&mut self, t: &AssignmentTarget<'_>) {
        use oxc_ast::ast::{AssignmentTargetMaybeDefault as ATMD, AssignmentTargetProperty as ATP};
        let maybe_default = |w: &mut Self, el: &ATMD<'_>| match el {
            ATMD::AssignmentTargetWithDefault(d) => {
                w.visit_target_reads(&d.binding);
                w.visit_expr(&d.init);
            }
            other => {
                if let Some(target) = other.as_assignment_target() {
                    w.visit_target_reads(target);
                }
            }
        };
        match t {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                self.record_ref_id(
                    id.name.as_str(),
                    id.span.start,
                    id.span.end,
                    RefParentKind::Read,
                );
            }
            AssignmentTarget::StaticMemberExpression(m) => self.visit_member_object(&m.object),
            AssignmentTarget::ComputedMemberExpression(m) => {
                self.visit_member_object(&m.object);
                self.visit_expr(&m.expression);
            }
            AssignmentTarget::PrivateFieldExpression(m) => self.visit_member_object(&m.object),
            AssignmentTarget::ArrayAssignmentTarget(arr) => {
                for el in arr.elements.iter().flatten() {
                    maybe_default(self, el);
                }
                if let Some(rest) = &arr.rest {
                    self.visit_target_reads(&rest.target);
                }
            }
            AssignmentTarget::ObjectAssignmentTarget(obj) => {
                for p in &obj.properties {
                    match p {
                        ATP::AssignmentTargetPropertyIdentifier(pi) => {
                            self.record_ref_id(
                                pi.binding.name.as_str(),
                                pi.binding.span.start,
                                pi.binding.span.end,
                                RefParentKind::Read,
                            );
                            if let Some(init) = &pi.init {
                                self.visit_expr(init);
                            }
                        }
                        ATP::AssignmentTargetPropertyProperty(pp) => {
                            self.visit_key(&pp.name, pp.computed);
                            maybe_default(self, &pp.binding);
                        }
                    }
                }
                if let Some(rest) = &obj.rest {
                    self.visit_target_reads(&rest.target);
                }
            }
            _ => {}
        }
    }

    fn visit_update(&mut self, u: &UpdateExpression<'_>) {
        // `foo++` / `foo.bar++`
        let target = &u.argument;
        if let Some(index) = self.reactive_statement {
            let root = match target {
                SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.name.as_str()),
                other => other
                    .as_member_expression()
                    .and_then(|m| member_root_identifier(m.object()))
                    .map(|id| id.name.as_str()),
            };
            if let Some(name) = root {
                let scope = self.cur_scope();
                self.tree.reactive_statements[index]
                    .assignments
                    .push((SmolStr::from(name), scope));
            }
        }
        if !self.state_fields.is_empty()
            && let Some(member) = target.as_member_expression()
            && matches!(member.object(), Expression::ThisExpression(_))
        {
            let name = match member {
                oxc_ast::ast::MemberExpression::StaticMemberExpression(m) => {
                    Some(m.property.name.to_string())
                }
                oxc_ast::ast::MemberExpression::PrivateFieldExpression(m) => {
                    Some(format!("#{}", m.field.name))
                }
                oxc_ast::ast::MemberExpression::ComputedMemberExpression(m) => {
                    literal_key_name(&m.expression)
                }
            };
            self.check_state_field_write(name, u.span);
        }
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = target {
            self.push_write(vec![SmolStr::from(id.name.as_str())], true, u.span);
        }
        match target {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                self.record_ref_id(
                    id.name.as_str(),
                    id.span.start,
                    id.span.end,
                    RefParentKind::UpdateTarget,
                );
                self.tree.pending_updates.push(PendingUpdate {
                    scope: self.cur_scope(),
                    name: SmolStr::from(id.name.as_str()),
                    range: self.abs(id.span.start, id.span.end),
                    is_reassign: true,
                });
            }
            SimpleAssignmentTarget::StaticMemberExpression(m) => {
                if let Some(base) = base_identifier(&m.object) {
                    self.record_ref_id(
                        base.0,
                        base.1,
                        base.2,
                        RefParentKind::MemberObjectOfAssignment,
                    );
                    self.tree.pending_updates.push(PendingUpdate {
                        scope: self.cur_scope(),
                        name: SmolStr::from(base.0),
                        range: self.abs(base.1, base.2),
                        is_reassign: false,
                    });
                }
                self.visit_expr(&m.object);
            }
            SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                if let Some(base) = base_identifier(&m.object) {
                    self.record_ref_id(
                        base.0,
                        base.1,
                        base.2,
                        RefParentKind::MemberObjectOfAssignment,
                    );
                    self.tree.pending_updates.push(PendingUpdate {
                        scope: self.cur_scope(),
                        name: SmolStr::from(base.0),
                        range: self.abs(base.1, base.2),
                        is_reassign: false,
                    });
                }
                self.visit_expr(&m.object);
                self.visit_expr(&m.expression);
            }
            _ => {}
        }
    }

    fn record_ref(&mut self, id: &IdentifierReference<'_>, parent_kind: RefParentKind) {
        self.record_ref_id(id.name.as_str(), id.span.start, id.span.end, parent_kind);
    }

    fn record_ref_id(&mut self, name: &str, start: u32, end: u32, parent_kind: RefParentKind) {
        self.record_ref_id_full(name, start, end, parent_kind, false);
    }

    fn record_ref_id_full(
        &mut self,
        name: &str,
        start: u32,
        end: u32,
        parent_kind: RefParentKind,
        parent_is_call: bool,
    ) {
        // `arguments` outside every function declaration / expression
        // (arrows have none of their own).
        if name == "arguments" && self.plain_function_depth == 0 && self.hooks.is_some() {
            let range = self.abs(start, end);
            self.push_error(
                Code::invalid_arguments_usage,
                crate::messages::invalid_arguments_usage(),
                range,
            );
        }
        let ignored = self.current_ignore_snapshot();
        self.tree.pending_refs.push(PendingRef {
            scope: self.cur_scope(),
            name: SmolStr::from(name),
            range: self.abs(start, end),
            parent_kind,
            function_depth_at_use: self.function_depth + self.rune_bump,
            nested_in_state_call: self.in_state_arg_nested,
            in_function_closure: self.in_function_closure,
            in_template: false,
            in_control_flow: false,
            is_bind_this: false,
            parent_is_call,
            in_reactive_statement: self.in_reactive_statement,
            ignored,
        });
    }
}

/// Patch template-context flags onto every `PendingRef` whose
/// byte-range is inside `slice` (pushed during the expression walk
/// that covers `slice`). Assumes refs are appended to the tail in
/// walk order.
fn apply_template_flags_since(
    refs: &mut [PendingRef],
    slice: Range,
    flags: RefFlags,
    in_template: bool,
    in_control_flow: bool,
) {
    // Walk the tail in reverse. Stop when we find a ref whose range
    // is strictly before `slice.start` — those were pushed before
    // this template walk began.
    for r in refs.iter_mut().rev() {
        if r.range.start < slice.start {
            break;
        }
        if r.range.start >= slice.start && r.range.end <= slice.end {
            r.in_template = in_template;
            r.in_control_flow = in_control_flow;
            r.is_bind_this = flags.is_bind_this;
        }
    }
}

/// The identifiers an assignment target writes, in pattern order —
/// upstream `extract_identifiers` over the assignment's left side.
/// Member targets (`a.b = …`) contribute nothing.
fn assignment_target_identifiers<'a>(
    t: &'a AssignmentTarget<'a>,
    out: &mut Vec<(&'a str, u32, u32)>,
) {
    use oxc_ast::ast::{AssignmentTargetMaybeDefault as ATMD, AssignmentTargetProperty as ATP};
    match t {
        AssignmentTarget::AssignmentTargetIdentifier(id) => {
            out.push((id.name.as_str(), id.span.start, id.span.end));
        }
        AssignmentTarget::ArrayAssignmentTarget(arr) => {
            for el in arr.elements.iter().flatten() {
                assignment_target_maybe_default_identifiers(el, out);
            }
            if let Some(rest) = &arr.rest {
                assignment_target_identifiers(&rest.target, out);
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(obj) => {
            for p in &obj.properties {
                match p {
                    ATP::AssignmentTargetPropertyIdentifier(pi) => {
                        out.push((
                            pi.binding.name.as_str(),
                            pi.binding.span.start,
                            pi.binding.span.end,
                        ));
                    }
                    ATP::AssignmentTargetPropertyProperty(pp) => {
                        assignment_target_maybe_default_identifiers(&pp.binding, out);
                    }
                }
            }
            if let Some(rest) = &obj.rest {
                assignment_target_identifiers(&rest.target, out);
            }
        }
        _ => {}
    }
    fn assignment_target_maybe_default_identifiers<'a>(
        t: &'a ATMD<'a>,
        out: &mut Vec<(&'a str, u32, u32)>,
    ) {
        match t {
            ATMD::AssignmentTargetWithDefault(d) => assignment_target_identifiers(&d.binding, out),
            other => {
                if let Some(target) = other.as_assignment_target() {
                    assignment_target_identifiers(target, out);
                }
            }
        }
    }
}
