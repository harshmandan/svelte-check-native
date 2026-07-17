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
    ChainElement, Class, ClassBody, ClassElement, Expression, ForStatementInit,
    IdentifierReference, LabeledStatement, ObjectExpression, ObjectPattern, ObjectPropertyKind,
    Program, PropertyKey, SimpleAssignmentTarget, Statement, UpdateExpression, VariableDeclaration,
    VariableDeclarator,
};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_core::Range;

use svn_parser::document::{Document, ScriptSection};
use svn_parser::parse_script_body;

pub use crate::scope_rune_detection::is_rune_name;
use crate::scope_rune_detection::{
    detect_bindable_default, detect_rune_call_from_call, is_primitive_expr, is_primitive_rune_init,
    state_rune_primitive_arg,
};
use crate::scope_util::{
    base_identifier, expression_from_default, expression_from_for_init,
    expression_from_property_key, extract_base_ident, idents_in_pattern, unwrap_ts_wrappers,
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
    /// scope wins (innermost).
    pub fn innermost_template_scope_at(&self, offset: u32) -> ScopeId {
        let mut best = self.instance_root;
        let mut best_len = u32::MAX;
        for (i, s) in self.scopes.iter().enumerate() {
            if let Some(r) = s.range
                && r.start <= offset
                && offset < r.end
                && (r.end - r.start) < best_len
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
pub fn build_with_template_and_runes(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    runes: bool,
    compat: crate::compat::CompatFeatures,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
) -> ScopeTree {
    let mut tree = build_with_template(
        doc,
        fragment,
        source,
        runes,
        module_program,
        instance_program,
    );
    if !runes {
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
    for binding in &mut tree.bindings {
        binding.fires_state_referenced_locally = match binding.kind {
            BindingKind::RawState | BindingKind::Derived => true,
            BindingKind::Prop => compat.state_locally_fires_on_props,
            BindingKind::RestProp => {
                compat.state_locally_fires_on_props && compat.state_locally_rest_prop
            }
            BindingKind::State => binding.reassigned || is_primitive_rune_init(&binding.initial),
            _ => false,
        };
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
pub fn build_with_template(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    runes: bool,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
) -> ScopeTree {
    let mut tree_builder = TreeBuilder::new();

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
        let template_root = tree_builder.new_scope(Some(instance_root));
        // Stamp the whole-fragment range so top-level template positions
        // (e.g. a top-level `{@const}` declaration) resolve to
        // `template_root` via `innermost_template_scope_at` (l275).
        tree_builder.scopes[template_root.0 as usize].range = Some(frag.range);
        let lang = doc
            .instance_script
            .as_ref()
            .map(|s| s.lang)
            .unwrap_or(svn_parser::document::ScriptLang::Js);
        tree_builder.walk_template(frag, source, template_root, lang);
    }

    tree_builder.finish(module_root, instance_root)
}

struct TreeBuilder {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    /// Pass-2 work queue: (scope, name, offset-within-script, base_offset,
    /// parent_kind, function_depth_at_use, nested_in_state, in_fn_closure).
    pending_refs: Vec<PendingRef>,
    pending_updates: Vec<PendingUpdate>,
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
    /// Arena reused across the per-expression template mini-parses in
    /// `walk_expr_range`. Templates carry hundreds of tiny `{expr}`
    /// slices per file; constructing a fresh oxc Allocator for each
    /// costs more than the parse itself, so one arena is taken out of
    /// this slot per parse, `reset()` (which keeps its largest chunk),
    /// and put back. `None` only while a parse is in flight.
    expr_alloc: Option<oxc_allocator::Allocator>,
    /// See [`ScopeTree::has_await`].
    has_await: bool,
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
            custom_element_props_candidates: Vec::new(),
            custom_element_props_ignored: Vec::new(),
            nonrunes_export_idents: Vec::new(),
            nonrunes_export_specs: Vec::new(),
            expr_alloc: None,
            has_await: false,
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

    fn bindings_in(&self, scope: ScopeId) -> Vec<BindingId> {
        self.scopes[scope.0 as usize]
            .declarations
            .values()
            .copied()
            .collect()
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
                    self.record_template_ref(d.name.as_str(), d.range, ctx, RefFlags::default());
                }
                match &d.value {
                    Some(svn_parser::ast::DirectiveValue::Expression {
                        expression_range, ..
                    }) => {
                        self.walk_expr_range(*expression_range, ctx, flags);
                        if d.kind == DirectiveKind::Bind {
                            self.register_bind_update(*expression_range, ctx);
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
                self.walk_expr_range(abs, ctx, RefFlags::default());
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
        let needs_wrap = leading.and_then(|i| slice.as_bytes().get(i).copied()) == Some(b'{');
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
        };
        for stmt in &parsed.program.body {
            walker.visit_stmt(stmt);
        }
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
    fn register_bind_update(&mut self, range: Range, ctx: &mut TemplateCtx<'_>) {
        let Some(raw) = ctx.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let slice = raw.trim();
        if slice.is_empty() {
            return;
        }
        // Bare identifier → also push a reassignment for
        // `non_reactive_update`.
        if slice
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            self.pending_updates.push(PendingUpdate {
                scope: ctx.scope,
                name: SmolStr::from(slice),
                range,
                is_reassign: true,
            });
        }
        // Extract the base identifier for member / call chains too —
        // `rest[0]` and `rest.foo` both root on `rest`.
        if let Some(base) = extract_base_ident(slice)
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
    fn enter_scope(
        &mut self,
        kind: svn_analyze::template_scope::ScopeKind,
        bindings: &[svn_analyze::template_scope::BoundIdent],
        scope_range: svn_core::Range,
    ) {
        use svn_analyze::template_scope::ScopeKind;
        let parent = self.ctx.scope;
        let child = self.builder.new_scope(Some(parent));
        // Record the scope's lexical span so an on-event reference
        // resolves against the element's scope, not the whole-file
        // declaration set (l275).
        self.builder.scopes[child.0 as usize].range = Some(scope_range);
        self.scope_stack.push(parent);
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
        let (declare_kind, retag_to_template) = match kind {
            ScopeKind::Each { .. } => (BindingKind::Each, false),
            ScopeKind::AwaitThen | ScopeKind::AwaitCatch => (BindingKind::Template, false),
            ScopeKind::Snippet => (BindingKind::Each, true),
            ScopeKind::LetDirective => (BindingKind::Each, true),
            ScopeKind::Fragment => unreachable!("walker doesn't call enter_scope for Fragment"),
        };

        let context_count = match kind {
            ScopeKind::Each { has_index, .. } if has_index => bindings.len().saturating_sub(1),
            _ => bindings.len(),
        };
        let context_bindings = &bindings[..context_count];
        for b in context_bindings {
            let bid = self.builder.declare(
                child,
                b.name.clone(),
                b.range,
                declare_kind,
                DeclarationKind::Const,
                InitialKind::EachBlock,
            );
            self.builder.bindings[bid.0 as usize].inside_rest = b.inside_rest;
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
            self.builder.declare(
                child,
                index.name.clone(),
                index.range,
                index_kind,
                DeclarationKind::Const,
                InitialKind::EachBlock,
            );
        }

        if retag_to_template {
            // Snippet params and let-directive bindings are declared
            // with `BindingKind::Each` above (so the shared declarer
            // path stays uniform), then retagged to `Template` to
            // match upstream's per-kind classification. Mirrors what
            // the pre-Phase-4 `declare_snippet_params` /
            // `declare_let_directive` did.
            for bid in self.builder.bindings_in(child) {
                if matches!(
                    self.builder.bindings[bid.0 as usize].kind,
                    BindingKind::Each
                ) {
                    self.builder.bindings[bid.0 as usize].kind = BindingKind::Template;
                }
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
        };
        // Push the template-comment ignores so they apply to every
        // reference recorded during this script walk.
        if !leading_ignores.is_empty() {
            walker.ignore_frames.push(leading_ignores.to_vec());
        }
        for stmt in &program.body {
            walker.at_program_top = true;
            walker.visit_stmt(stmt);
        }
        if !leading_ignores.is_empty() {
            walker.ignore_frames.pop();
        }
        // Harvest non-runes `export` facts from the instance script's
        // top level — so the later `promote_non_runes_exports` pass
        // reads owned `SmolStr` facts instead of re-parsing the entire
        // instance body. Names only; the resolve / `Var|Let` gate runs
        // later against the built tree. Mirrors the old re-parse's
        // statement matching exactly.
        if is_instance {
            for stmt in &program.body {
                let Statement::ExportNamedDeclaration(end) = stmt else {
                    continue;
                };
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(v)) = &end.declaration {
                    // `export const` doesn't become a prop.
                    if matches!(v.kind, oxc_ast::ast::VariableDeclarationKind::Const) {
                        continue;
                    }
                    for d in &v.declarations {
                        for name in idents_in_pattern(&d.id) {
                            self.nonrunes_export_idents.push(SmolStr::from(name));
                        }
                    }
                } else if end.declaration.is_none() {
                    for spec in &end.specifiers {
                        use oxc_ast::ast::ModuleExportName;
                        let local = match &spec.local {
                            ModuleExportName::IdentifierName(id) => id.name.as_str(),
                            ModuleExportName::IdentifierReference(id) => id.name.as_str(),
                            ModuleExportName::StringLiteral(_) => continue,
                        };
                        let exported = match &spec.exported {
                            ModuleExportName::IdentifierName(id) => Some(id.name.as_str()),
                            ModuleExportName::IdentifierReference(id) => Some(id.name.as_str()),
                            ModuleExportName::StringLiteral(_) => None,
                        };
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

        synthesize_store_subs(&mut self, &mut unresolved, module_root, instance_root);

        ScopeTree {
            scopes: self.scopes,
            bindings: self.bindings,
            module_root,
            instance_root,
            unresolved_refs: unresolved,
            has_await: self.has_await,
            custom_element_props_candidates: self.custom_element_props_candidates,
            custom_element_props_ignored: self.custom_element_props_ignored,
            nonrunes_export_idents: self.nonrunes_export_idents,
            nonrunes_export_specs: self.nonrunes_export_specs,
        }
    }
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
    module_root: ScopeId,
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
        let backing = resolve_by_name(&tree.scopes, module_root, store_name)
            .or_else(|| resolve_by_name(&tree.scopes, instance_root, store_name));
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
            if let InitialKind::RuneCall { rune, .. } = backing_binding.initial {
                let props_exception = store_name != "props" && rune == RuneCall::Props;
                if !props_exception {
                    continue;
                }
            }
            // Additional carve-out: `let { props } = $props()` destructures
            // a field NAMED `props` out of the rune call. Our destructure
            // handler rewires the field's `initial` to `None` (matching
            // upstream `VariableDeclarator.js:104-130`), so the
            // `RuneCall { Props }` check above doesn't catch it. The
            // destructured `props` field carries `BindingKind::Prop`,
            // which is unique to fields destructured from `$props()` —
            // that's the signal we use to skip synthesis here.
            //
            // Without this skip, `let { props } = $props()` fires a
            // false-positive `store_rune_conflict` on every reference to
            // `$props(...)` in the script — verified against upstream
            // svelte/compiler 5.53.6 on threlte/theatre, which does NOT
            // fire the warning for this canonical pattern.
            if store_name == "props" && backing_binding.kind == BindingKind::Prop {
                continue;
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
    }

    fn visit_stmt(&mut self, stmt: &Statement<'_>) {
        let pushed = self.push_leading_ignores(Some(stmt.span().start));
        // Only the Program-body loop sets the flag; every statement
        // visited from here down is nested.
        let at_program_top = std::mem::replace(&mut self.at_program_top, false);
        self.visit_stmt_inner(stmt, at_program_top);
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
                    if let Some(body) = &f.body {
                        for s in &body.statements {
                            w.visit_stmt(s);
                        }
                    }
                });
            }
            Statement::ClassDeclaration(cls) => self.visit_class_decl(cls),
            Statement::ImportDeclaration(imp) => {
                let source = SmolStr::from(imp.source.value.as_str());
                if let Some(specs) = &imp.specifiers {
                    for s in specs {
                        use oxc_ast::ast::ImportDeclarationSpecifier as S;
                        let (name, span, is_default) = match s {
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
            }
            Statement::ExportNamedDeclaration(end) => {
                if let Some(decl) = &end.declaration {
                    // Re-wrap as a statement-like visit.
                    use oxc_ast::ast::Declaration;
                    match decl {
                        Declaration::VariableDeclaration(v) => self.visit_var_decl(v),
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
                                if let Some(body) = &f.body {
                                    for s in &body.statements {
                                        w.visit_stmt(s);
                                    }
                                }
                            });
                        }
                        Declaration::ClassDeclaration(cls) => self.visit_class_decl(cls),
                        _ => {}
                    }
                }
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
                for s in &t.block.body {
                    self.visit_stmt(s);
                }
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
                    for s in &f.body {
                        self.visit_stmt(s);
                    }
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
            Statement::ExpressionStatement(es) => self.visit_expr(&es.expression),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.visit_expr(arg);
                }
            }
            Statement::LabeledStatement(lbl) => self.visit_labeled(lbl, at_program_top),
            Statement::ThrowStatement(t) => self.visit_expr(&t.argument),
            Statement::ExportDefaultDeclaration(ed) => {
                use oxc_ast::ast::ExportDefaultDeclarationKind as Ed;
                match &ed.declaration {
                    Ed::FunctionDeclaration(f) => {
                        self.with_function(|w| {
                            for p in &f.params.items {
                                w.declare_pattern(&p.pattern, DeclarationKind::Param);
                            }
                            if let Some(body) = &f.body {
                                for s in &body.statements {
                                    w.visit_stmt(s);
                                }
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

    fn visit_labeled(&mut self, lbl: &LabeledStatement<'_>, at_program_top: bool) {
        // `$: …` — upstream puts the LHS name into
        // `possible_implicit_declarations` and promotes it to
        // `legacy_reactive` post-walk if no outer binding exists.
        // Not ported yet. For
        // `reactive_declaration_module_script_dependency` we need to
        // know the reference sits inside a `$:` block at the top
        // level of the instance script.
        let is_top_level_reactive = lbl.label.name == "$" && self.is_instance && at_program_top;
        if is_top_level_reactive {
            let prev = std::mem::replace(&mut self.in_reactive_statement, true);
            self.visit_stmt(&lbl.body);
            self.in_reactive_statement = prev;
        } else {
            self.visit_stmt(&lbl.body);
        }
    }

    fn visit_class_decl(&mut self, cls: &Class<'_>) {
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
        if let Some(sup) = &cls.super_class {
            self.visit_expr(sup);
        }
        self.visit_class_body(&cls.body);
    }

    fn visit_class_body(&mut self, body: &ClassBody<'_>) {
        for m in &body.body {
            let pushed = self.push_leading_ignores(Some(m.span().start));
            match m {
                ClassElement::MethodDefinition(md) => {
                    if md.computed
                        && let Some(k) = expression_from_property_key(&md.key)
                    {
                        self.visit_expr(k);
                    }
                    self.with_function(|w| {
                        if let Some(body) = &md.value.body {
                            for p in &md.value.params.items {
                                w.declare_pattern(&p.pattern, DeclarationKind::Param);
                            }
                            for s in &body.statements {
                                w.visit_stmt(s);
                            }
                        }
                    });
                }
                ClassElement::PropertyDefinition(p) => {
                    if p.computed
                        && let Some(k) = expression_from_property_key(&p.key)
                    {
                        self.visit_expr(k);
                    }
                    if let Some(v) = &p.value {
                        self.visit_expr(v);
                    }
                }
                ClassElement::AccessorProperty(p) => {
                    if p.computed
                        && let Some(k) = expression_from_property_key(&p.key)
                    {
                        self.visit_expr(k);
                    }
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
        self.visit_declarator_inner(d, decl_kind);
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
                    .unwrap_or(true);
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
                    .unwrap_or(true);
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
                    primitive_arg: false,
                },
            ),
            Some(RuneCall::DerivedBy) => (
                BindingKind::Derived,
                InitialKind::RuneCall {
                    rune: RuneCall::DerivedBy,
                    primitive_arg: false,
                },
            ),
            Some(RuneCall::Props) => (
                BindingKind::Prop,
                InitialKind::RuneCall {
                    rune: RuneCall::Props,
                    primitive_arg: false,
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
            self.visit_expr(init);
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
                            primitive_arg: primitive,
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
        self.visit_expr_inner(e);
        if pushed {
            self.ignore_frames.pop();
        }
    }

    fn visit_expr_inner(&mut self, e: &Expression<'_>) {
        match e {
            Expression::Identifier(id) => self.record_ref(id, RefParentKind::Read),
            Expression::ArrowFunctionExpression(arr) => {
                self.with_function(|w| {
                    for p in &arr.params.items {
                        w.declare_pattern(&p.pattern, DeclarationKind::Param);
                    }
                    for s in &arr.body.statements {
                        w.visit_stmt(s);
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
                    if let Some(body) = &f.body {
                        for s in &body.statements {
                            w.visit_stmt(s);
                        }
                    }
                });
            }
            Expression::CallExpression(c) => self.visit_call(c),
            Expression::NewExpression(n) => {
                self.visit_expr(&n.callee);
                for a in &n.arguments {
                    self.visit_argument(a, false);
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
            Expression::TemplateLiteral(t) => {
                for e in &t.expressions {
                    self.visit_expr(e);
                }
            }
            Expression::TaggedTemplateExpression(t) => {
                self.visit_expr(&t.tag);
                for e in &t.quasi.expressions {
                    self.visit_expr(e);
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
            Expression::StaticMemberExpression(m) => self.visit_member_object(&m.object),
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
                    if op.computed {
                        if let PropertyKey::StaticIdentifier(_) = &op.key {
                            // ignore
                        } else if let Some(e) = expression_from_property_key(&op.key) {
                            self.visit_expr(e);
                        }
                    }
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
        let bump = matches!(
            rune,
            Some(RuneCall::Derived) | Some(RuneCall::DerivedBy) | Some(RuneCall::Inspect)
        );
        // Track `nested_in_state_call` for refs inside arg subtrees —
        // used by state_referenced_locally's message discriminator.
        let push_state = matches!(rune, Some(RuneCall::State) | Some(RuneCall::StateRaw));
        // Callee — flag the identifier (if any) as being the callee
        // of a CallExpression for `store_rune_conflict`'s sake.
        self.visit_callee(&c.callee);
        if bump {
            self.rune_bump += 1;
        }
        for a in &c.arguments {
            self.visit_argument(a, push_state);
        }
        if bump {
            self.rune_bump -= 1;
        }
    }

    /// One call/new argument. `as_expression()` is None for
    /// `Argument::SpreadElement`, so spreads need their own arm —
    /// `f(...props)` must record the references inside the spread
    /// argument (verified: upstream counts them, and a `$state` read
    /// inside `f(...[count])` fires `state_referenced_locally`).
    fn visit_argument(&mut self, a: &oxc_ast::ast::Argument<'_>, in_state_call: bool) {
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
            Expression::Identifier(id) => self.record_ref(id, RefParentKind::Read),
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

    fn visit_assignment(&mut self, a: &AssignmentExpression<'_>) {
        // Record the target.
        self.visit_assignment_target(&a.left);
        self.visit_expr(&a.right);
    }

    fn visit_assignment_target(&mut self, t: &AssignmentTarget<'_>) {
        match t {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                // foo = …
                self.record_ref_id(
                    id.name.as_str(),
                    id.span.start,
                    id.span.end,
                    RefParentKind::AssignmentLeft,
                );
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
                    self.visit_assignment_target(&rest.target);
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
                                RefParentKind::AssignmentLeft,
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
                            if pp.computed
                                && let Some(k) = expression_from_property_key(&pp.name)
                            {
                                self.visit_expr(k);
                            }
                            self.visit_assignment_target_maybe_default(&pp.binding);
                        }
                    }
                }
                if let Some(rest) = &obj.rest {
                    self.visit_assignment_target(&rest.target);
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
                self.visit_assignment_target(&d.binding);
                self.visit_expr(&d.init);
            }
            other => {
                if let Some(target) = other.as_assignment_target() {
                    self.visit_assignment_target(target);
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
                            if pp.computed
                                && let Some(k) = expression_from_property_key(&pp.name)
                            {
                                self.visit_expr(k);
                            }
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
