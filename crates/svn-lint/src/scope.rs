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
    ArrayPattern, AssignmentExpression, AssignmentTarget, BindingPattern, BindingPatternKind,
    CallExpression, ChainElement, Class, ClassBody, ClassElement, Expression, ForStatementInit,
    IdentifierReference, LabeledStatement, ObjectExpression, ObjectPattern, ObjectPropertyKind,
    PropertyKey, SimpleAssignmentTarget, Statement, UpdateExpression, VariableDeclaration,
    VariableDeclarator,
};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use std::collections::HashMap;
use svn_core::Range;

use svn_parser::document::{Document, ScriptSection};
use svn_parser::parse_script_body;

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
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub function_depth: u32,
    pub declarations: HashMap<SmolStr, BindingId>,
}

impl Scope {
    fn new(parent: Option<ScopeId>, function_depth: u32) -> Self {
        Self {
            parent,
            function_depth,
            declarations: HashMap::new(),
        }
    }
}

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
    /// rules like `store_rune_conflict` can inspect them.
    pub unresolved_refs: Vec<UnresolvedRef>,
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
        let mut cur = Some(from);
        while let Some(sid) = cur {
            let s = self.scope(sid);
            if let Some(&bid) = s.declarations.get(name) {
                return Some(bid);
            }
            cur = s.parent;
        }
        None
    }

    /// Like [`resolve`], but resolves against both the instance root
    /// and module root — used by template walkers that don't have a
    /// script-local scope to start from.
    pub fn resolve_from_template(&self, name: &str) -> Option<BindingId> {
        self.resolve(self.instance_root, name)
    }

    /// True when `name` is declared in *any* scope — module root,
    /// instance root, or a template-side snippet / each-block scope.
    /// Used by rules like `attribute_global_event_reference` that
    /// only need to know "is this identifier bound somewhere?" at
    /// the point of use, without threading the current template
    /// scope through the rule signature.
    pub fn is_declared_anywhere(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .any(|s| s.declarations.contains_key(name))
    }
}

/// Build a scope tree for the whole document.
///
/// Module script sits at `function_depth = 0`; instance script has
/// the module scope as its parent, giving `function_depth = 1` for
/// the instance root — matching upstream exactly.
pub fn build(doc: &Document<'_>) -> ScopeTree {
    build_with_template(doc, None, "")
}

/// Should the file be treated as runes mode for post-walk bookkeeping?
/// Caller controls this so it matches `ctx.runes` at the rules layer.
pub fn build_with_template_and_runes(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
    runes: bool,
) -> ScopeTree {
    let mut tree = build_with_template(doc, fragment, source);
    if !runes {
        promote_non_runes_exports(&mut tree, doc);
    }
    tree
}

fn promote_non_runes_exports(tree: &mut ScopeTree, doc: &Document<'_>) {
    let Some(script) = &doc.instance_script else {
        return;
    };
    let alloc = oxc_allocator::Allocator::default();
    let parsed = parse_script_body(&alloc, script.content, script.lang);
    for stmt in &parsed.program.body {
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
                    promote_to_bindable_prop(tree, tree.instance_root, &name);
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
                let Some(bid) = tree.resolve(tree.instance_root, local) else {
                    continue;
                };
                let b = &tree.bindings[bid.0 as usize];
                if matches!(
                    b.declaration_kind,
                    DeclarationKind::Var | DeclarationKind::Let
                ) {
                    promote_to_bindable_prop(tree, tree.instance_root, local);
                    let exported = match &spec.exported {
                        ModuleExportName::IdentifierName(id) => Some(id.name.as_str()),
                        ModuleExportName::IdentifierReference(id) => Some(id.name.as_str()),
                        ModuleExportName::StringLiteral(_) => None,
                    };
                    if let Some(alias) = exported
                        && alias != local
                    {
                        tree.bindings[bid.0 as usize].prop_alias = Some(SmolStr::from(alias));
                    }
                }
            }
        }
    }
    drop(parsed);
    drop(alloc);
}

fn promote_to_bindable_prop(tree: &mut ScopeTree, root: ScopeId, name: &str) {
    if let Some(bid) = tree.resolve(root, name) {
        let b = &mut tree.bindings[bid.0 as usize];
        b.kind = BindingKind::BindableProp;
    }
}

fn idents_in_pattern(pat: &BindingPattern<'_>) -> Vec<String> {
    let mut out = Vec::new();
    fn go(pat: &BindingPattern<'_>, out: &mut Vec<String>) {
        match &pat.kind {
            BindingPatternKind::BindingIdentifier(id) => out.push(id.name.to_string()),
            BindingPatternKind::ObjectPattern(op) => {
                for prop in &op.properties {
                    go(&prop.value, out);
                }
                if let Some(rest) = &op.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPatternKind::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    go(p, out);
                }
                if let Some(rest) = &ap.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPatternKind::AssignmentPattern(ap) => go(&ap.left, out),
        }
    }
    go(pat, &mut out);
    out
}

/// Scan backward from `script_start` through whitespace-only runs
/// of template text looking for one or more consecutive
/// `<!-- svelte-ignore CODE, CODE, … -->` comments, and return the
/// flattened list of codes. Mirrors upstream's treatment of
/// comment-siblings immediately preceding a node in the root
/// fragment — the codes silence fires inside the following node
/// (which for us is the instance `<script>`).
fn collect_preceding_template_ignores(source: &str, script_start: u32) -> Vec<SmolStr> {
    let bytes = source.as_bytes();
    let mut end = script_start as usize;
    let mut codes: Vec<SmolStr> = Vec::new();
    loop {
        // Skip whitespace backward.
        while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        // Must see `-->` now.
        if end < 3 || &bytes[end - 3..end] != b"-->" {
            break;
        }
        // Find matching `<!--` (search backward).
        let Some(open) = source[..end - 3].rfind("<!--") else {
            break;
        };
        let body = &source[open + 4..end - 3];
        let trimmed = body.trim_start();
        let Some(rest) = trimmed.strip_prefix("svelte-ignore") else {
            break;
        };
        let rest = match rest.chars().next() {
            Some(ch) if ch.is_whitespace() => &rest[ch.len_utf8()..],
            _ => break,
        };
        // Parse codes in runes-mode lenient (same path script
        // leading-comments use). Prepend so the scan order mirrors
        // source order.
        let comment_codes = crate::ignore::parse_ignore_codes_public(rest, true);
        let mut merged = comment_codes;
        merged.extend(codes);
        codes = merged;
        end = open;
    }
    codes
}

/// Like [`build`], but also walks the template fragment — capturing
/// references in attribute expressions / interpolations / directive
/// values and the implicit reassignments from `bind:*` directives.
/// Callers that only need script-side information can use [`build`].
pub fn build_with_template(
    doc: &Document<'_>,
    fragment: Option<&svn_parser::ast::Fragment>,
    source: &str,
) -> ScopeTree {
    let mut tree_builder = TreeBuilder::new();

    // Module scope: if there's no module script at all we still create
    // a synthetic empty one so resolve() has a stable root. Matches
    // upstream's behavior — `create_scopes` always returns a scope
    // even for an empty Program body.
    let module_root = tree_builder.new_scope(None);
    if let Some(script) = &doc.module_script {
        tree_builder.build_script(script, module_root);
    }

    let instance_root = tree_builder.new_scope(Some(module_root));
    if let Some(script) = &doc.instance_script {
        // A `<!-- svelte-ignore CODE -->` comment placed in the
        // template immediately before `<script>` applies its codes
        // to the whole instance-script body. Upstream wires this up
        // naturally because the script is an AST sibling inside the
        // root Fragment; our sections parser extracts it separately,
        // so we have to bridge the ignore forward explicitly.
        let leading = collect_preceding_template_ignores(doc.source, script.open_tag_range.start);
        tree_builder.build_script_as_instance(script, instance_root, &leading);
    }

    if let Some(frag) = fragment {
        // Upstream template scope is a non-porous child of the
        // instance scope → function_depth = instance + 1. Mirror that
        // so template refs don't look like "same function_depth" as
        // instance-root bindings (important for
        // `state_referenced_locally`).
        let template_root = tree_builder.new_scope(Some(instance_root));
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
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let depth = match parent {
            Some(pid) => self.scopes[pid.0 as usize].function_depth + 1,
            None => 0,
        };
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope::new(parent, depth));
        id
    }

    fn new_porous_scope(&mut self, parent: ScopeId) -> ScopeId {
        let depth = self.scopes[parent.0 as usize].function_depth;
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope::new(Some(parent), depth));
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
            has_bind_reference: false,
        });
        self.scopes[scope.0 as usize].declarations.insert(name, id);
        id
    }

    /// Walk the template fragment, extracting every expression-bearing
    /// site and feeding it through a lightweight script-body-like
    /// walker. Reference flags (`in_template`, `in_control_flow`,
    /// `is_bind_this`) are threaded through so
    /// `non_reactive_update` can decide which references to trust.
    fn walk_template(
        &mut self,
        fragment: &svn_parser::ast::Fragment,
        source: &str,
        instance_root: ScopeId,
        lang: svn_parser::document::ScriptLang,
    ) {
        let mut ctx = TemplateCtx {
            source,
            scope: instance_root,
            lang,
            in_control_flow: false,
        };
        self.walk_template_fragment(fragment, &mut ctx);
    }

    fn walk_template_fragment(
        &mut self,
        fragment: &svn_parser::ast::Fragment,
        ctx: &mut TemplateCtx<'_>,
    ) {
        use svn_parser::ast::Node;
        for node in &fragment.nodes {
            match node {
                Node::Element(el) => {
                    self.walk_element_like(&el.attributes, &el.children, None, ctx);
                }
                Node::Component(comp) => {
                    let first_seg = comp.name.split('.').next().unwrap_or("");
                    self.walk_element_like(
                        &comp.attributes,
                        &comp.children,
                        Some((first_seg, comp.range)),
                        ctx,
                    );
                }
                Node::SvelteElement(se) => {
                    self.walk_element_like(&se.attributes, &se.children, None, ctx);
                }
                Node::IfBlock(b) => {
                    self.walk_expr_range(b.condition_range, ctx, RefFlags::default());
                    let saved = std::mem::replace(&mut ctx.in_control_flow, true);
                    self.walk_template_fragment(&b.consequent, ctx);
                    for arm in &b.elseif_arms {
                        self.walk_expr_range(arm.condition_range, ctx, RefFlags::default());
                        self.walk_template_fragment(&arm.body, ctx);
                    }
                    if let Some(else_body) = &b.alternate {
                        self.walk_template_fragment(else_body, ctx);
                    }
                    ctx.in_control_flow = saved;
                }
                Node::EachBlock(b) => {
                    self.walk_expr_range(b.expression_range, ctx, RefFlags::default());
                    // Create a child scope for the each-block body
                    // and declare the context pattern identifiers in
                    // it. Upstream models these as `BindingKind::Each`
                    // (context) and `Static`/`Template` (index).
                    let child_scope = self.new_scope(Some(ctx.scope));
                    let parent_scope = std::mem::replace(&mut ctx.scope, child_scope);
                    if let Some(as_clause) = &b.as_clause {
                        self.declare_each_context(
                            as_clause.context_range,
                            child_scope,
                            parent_scope,
                            ctx.source,
                            ctx.lang,
                        );
                        if let Some(index) = as_clause.index_range {
                            let kind = if as_clause.key_range.is_some() {
                                BindingKind::Template
                            } else {
                                BindingKind::Static
                            };
                            let name = ctx
                                .source
                                .get(index.start as usize..index.end as usize)
                                .unwrap_or("")
                                .trim();
                            if !name.is_empty() {
                                self.declare(
                                    child_scope,
                                    SmolStr::from(name),
                                    index,
                                    kind,
                                    DeclarationKind::Const,
                                    InitialKind::EachBlock,
                                );
                            }
                        }
                        if let Some(key) = as_clause.key_range {
                            self.walk_expr_range(key, ctx, RefFlags::default());
                        }
                    }
                    let saved = std::mem::replace(&mut ctx.in_control_flow, true);
                    self.walk_template_fragment(&b.body, ctx);
                    if let Some(alternate) = &b.alternate {
                        self.walk_template_fragment(alternate, ctx);
                    }
                    ctx.in_control_flow = saved;
                    ctx.scope = parent_scope;
                }
                Node::AwaitBlock(b) => {
                    self.walk_expr_range(b.expression_range, ctx, RefFlags::default());
                    let saved = std::mem::replace(&mut ctx.in_control_flow, true);
                    if let Some(pending) = &b.pending {
                        self.walk_template_fragment(pending, ctx);
                    }
                    if let Some(then) = &b.then_branch {
                        self.walk_template_fragment(&then.body, ctx);
                    }
                    if let Some(catch) = &b.catch_branch {
                        self.walk_template_fragment(&catch.body, ctx);
                    }
                    ctx.in_control_flow = saved;
                }
                Node::KeyBlock(b) => {
                    self.walk_expr_range(b.expression_range, ctx, RefFlags::default());
                    let saved = std::mem::replace(&mut ctx.in_control_flow, true);
                    self.walk_template_fragment(&b.body, ctx);
                    ctx.in_control_flow = saved;
                }
                Node::SnippetBlock(b) => {
                    // `{#snippet name(p1, p2)}…{/snippet}` — each
                    // parameter is an implicit template-scope
                    // binding visible inside the body. Without
                    // declaring them, rules like
                    // `attribute_global_event_reference` consulting
                    // `is_declared_anywhere(name)` miss them, and
                    // references (`{p1}`) resolve against the outer
                    // scope (likely unresolved).
                    let snippet_scope = self.new_scope(Some(ctx.scope));
                    if b.parameters_range.start < b.parameters_range.end {
                        self.declare_snippet_params(
                            b.parameters_range,
                            snippet_scope,
                            ctx.source,
                            ctx.lang,
                        );
                    }
                    let saved = std::mem::replace(&mut ctx.scope, snippet_scope);
                    self.walk_template_fragment(&b.body, ctx);
                    ctx.scope = saved;
                }
                Node::Interpolation(intr) => {
                    // `{@const NAME = EXPR}` is a declaration, not an
                    // assignment — svn-parser strips the `@const `
                    // keyword before setting `expression_range`, so
                    // the naive walk parses it as `NAME = EXPR`
                    // (AssignmentExpression) and marks the outer
                    // `NAME` binding as reassigned. Detect the
                    // `{@const …}` form at the full-range source and
                    // re-parse as `let NAME = EXPR` so the binding
                    // lives in the current (template) scope.
                    let full = ctx
                        .source
                        .get(intr.range.start as usize..intr.range.end as usize)
                        .unwrap_or("");
                    if full.starts_with("{@const") {
                        self.walk_const_tag(intr.expression_range, ctx);
                    } else {
                        self.walk_expr_range(intr.expression_range, ctx, RefFlags::default());
                    }
                }
                Node::Text(_) | Node::Comment(_) => {}
            }
        }
    }

    fn walk_element_like(
        &mut self,
        attrs: &[svn_parser::ast::Attribute],
        children: &svn_parser::ast::Fragment,
        component_ref: Option<(&str, Range)>,
        ctx: &mut TemplateCtx<'_>,
    ) {
        use svn_parser::ast::{Attribute, DirectiveKind};
        // Record a reference for component tags like <Foo> /
        // <Foo.Bar> / <svelte:self> (caller supplies the first
        // identifier segment). Lets `export_let_unused` correctly
        // see the binding as referenced.
        if let Some((name, range)) = component_ref
            && !name.is_empty()
        {
            self.record_template_ref(name, range, ctx, RefFlags::default());
        }
        // Split attrs into let:directives (scoped to children) vs
        // everything else (scoped to current scope).
        let has_let = attrs
            .iter()
            .any(|a| matches!(a, Attribute::Directive(d) if d.kind == DirectiveKind::Let));
        for attr in attrs {
            if matches!(attr, Attribute::Directive(d) if d.kind == DirectiveKind::Let) {
                continue;
            }
            self.walk_template_attr(attr, ctx);
        }
        if has_let {
            let child_scope = self.new_scope(Some(ctx.scope));
            let parent_scope = std::mem::replace(&mut ctx.scope, child_scope);
            for attr in attrs {
                if let Attribute::Directive(d) = attr
                    && d.kind == DirectiveKind::Let
                {
                    self.declare_let_directive(d, child_scope, parent_scope, ctx.source, ctx.lang);
                }
            }
            self.walk_template_fragment(children, ctx);
            ctx.scope = parent_scope;
        } else {
            self.walk_template_fragment(children, ctx);
        }
    }

    /// `let:a` / `let:a={PATTERN}` on a component/svelte-fragment.
    /// Shorthand form (no `={…}`) declares a binding with the
    /// directive NAME. Expression form declares identifiers from
    /// PATTERN.
    fn declare_let_directive(
        &mut self,
        d: &svn_parser::ast::Directive,
        child_scope: ScopeId,
        parent_scope: ScopeId,
        source: &str,
        lang: svn_parser::document::ScriptLang,
    ) {
        use svn_parser::ast::DirectiveValue;
        match &d.value {
            None => {
                // `let:a` — shorthand, declares a binding named `a`.
                self.declare(
                    child_scope,
                    SmolStr::from(d.name.as_str()),
                    d.range,
                    BindingKind::Template,
                    DeclarationKind::Const,
                    InitialKind::None,
                );
            }
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                // Reuse the each-context parser — same pattern shape
                // semantics (patterns with possible destructures and
                // defaults).
                self.declare_each_context(
                    *expression_range,
                    child_scope,
                    parent_scope,
                    source,
                    lang,
                );
                // Re-tag declared bindings as Template (they were
                // declared as Each by declare_each_context).
                for bid in self.bindings_in(child_scope) {
                    self.bindings[bid.0 as usize].kind = BindingKind::Template;
                }
            }
            Some(_) => {}
        }
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
            // visit_assignment.
            self.declare_each_pattern(
                &d.id, ctx.scope, ctx.scope, offset, false, ctx.source, ctx.lang,
            );
            // Re-tag those bindings as Template (declare_each_pattern
            // uses `BindingKind::Each` internally).
            for bid in self.bindings_in(ctx.scope) {
                if matches!(self.bindings[bid.0 as usize].kind, BindingKind::Each) {
                    self.bindings[bid.0 as usize].kind = BindingKind::Template;
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
        let alloc = oxc_allocator::Allocator::default();
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
            type_annotation_depth: 0,
            in_state_arg_nested: false,
            in_reactive_statement: false,
            is_instance: false,
            program_depth: start_depth,
            // Template expression slices rarely carry
            // `// svelte-ignore` comments (they're inside `{…}`),
            // so skip the precollect for perf.
            script_ignore_comments: Vec::new(),
            script_content: effective_slice,
            ignore_frames: Vec::new(),
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
        drop(alloc);
    }

    /// Parse an `{#each ... as PATTERN}` context pattern (byte range
    /// relative to the source) and declare each identifier in
    /// `each_scope` with `BindingKind::Each`. Rest-element-nested
    /// identifiers get `inside_rest = true`. Default-value
    /// expressions (`{ a = expr }`) are walked in `parent_scope` so
    /// their references resolve to outer bindings — matches upstream
    /// scope.js:1244-1295.
    /// Declare each identifier in a snippet's `(param1, param2, …)`
    /// list into `snippet_scope` with kind `Template` — visible to
    /// the snippet body's template walk and to
    /// `ScopeTree::is_declared_anywhere` lookups. The slice passed
    /// in is the bytes INSIDE the parens (mirrors
    /// `SnippetBlock.parameters_range`). Handles destructures and
    /// rest-params — same pattern as each-blocks.
    fn declare_snippet_params(
        &mut self,
        range: Range,
        snippet_scope: ScopeId,
        source: &str,
        lang: svn_parser::document::ScriptLang,
    ) {
        let Some(slice) = source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let wrapped = format!("function _({slice}) {{}}");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, &wrapped, lang);
        if let Some(Statement::FunctionDeclaration(f)) = parsed.program.body.first() {
            // Prefix `function _(` is 11 bytes before params start.
            let offset: i32 = range.start as i32 - 11;
            for p in &f.params.items {
                self.declare_each_pattern(
                    &p.pattern,
                    snippet_scope,
                    snippet_scope,
                    offset,
                    false,
                    source,
                    lang,
                );
            }
            // Re-tag from `Each` to `Template` so the kind matches
            // the snippet semantics.
            for bid in self.bindings_in(snippet_scope) {
                self.bindings[bid.0 as usize].kind = BindingKind::Template;
            }
        }
        drop(parsed);
        drop(alloc);
    }

    fn declare_each_context(
        &mut self,
        range: Range,
        each_scope: ScopeId,
        parent_scope: ScopeId,
        source: &str,
        lang: svn_parser::document::ScriptLang,
    ) {
        let Some(slice) = source.get(range.start as usize..range.end as usize) else {
            return;
        };
        // Wrap the pattern so oxc can parse it as a destructuring
        // left-hand side. We use the shortest valid wrapper: a
        // VariableDeclaration with a dummy init.
        let wrapped = format!("let {slice} = 0;");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, &wrapped, lang);
        // Extract the declarator's id.
        if let Some(Statement::VariableDeclaration(vd)) = parsed.program.body.first()
            && let Some(d) = vd.declarations.first()
        {
            // The pattern was parsed with offsets relative to
            // `wrapped` (prefix "let " = 4 bytes). Translate back to
            // the original source.
            let offset: i32 = range.start as i32 - 4;
            self.declare_each_pattern(&d.id, each_scope, parent_scope, offset, false, source, lang);
        }
        drop(parsed);
        drop(alloc);
    }

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
        match &pat.kind {
            BindingPatternKind::BindingIdentifier(id) => {
                let abs_start = (id.span.start as i32 + offset).max(0) as u32;
                let abs_end = (id.span.end as i32 + offset).max(0) as u32;
                let bid = self.declare(
                    each_scope,
                    SmolStr::from(id.name.as_str()),
                    Range::new(abs_start, abs_end),
                    BindingKind::Each,
                    DeclarationKind::Const,
                    InitialKind::EachBlock,
                );
                self.bindings[bid.0 as usize].inside_rest = inside_rest;
            }
            BindingPatternKind::ObjectPattern(op) => {
                for prop in &op.properties {
                    self.declare_each_pattern(
                        &prop.value,
                        each_scope,
                        parent_scope,
                        offset,
                        inside_rest,
                        source,
                        lang,
                    );
                }
                if let Some(rest) = &op.rest {
                    self.declare_each_pattern(
                        &rest.argument,
                        each_scope,
                        parent_scope,
                        offset,
                        true,
                        source,
                        lang,
                    );
                }
            }
            BindingPatternKind::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    self.declare_each_pattern(
                        p,
                        each_scope,
                        parent_scope,
                        offset,
                        inside_rest,
                        source,
                        lang,
                    );
                }
                if let Some(rest) = &ap.rest {
                    self.declare_each_pattern(
                        &rest.argument,
                        each_scope,
                        parent_scope,
                        offset,
                        true,
                        source,
                        lang,
                    );
                }
            }
            BindingPatternKind::AssignmentPattern(ap) => {
                self.declare_each_pattern(
                    &ap.left,
                    each_scope,
                    parent_scope,
                    offset,
                    inside_rest,
                    source,
                    lang,
                );
                // Default-value expression — walk in the OUTER scope
                // so refs resolve to parent bindings, not the just-
                // declared each-block name.
                let abs = Range::new(
                    (ap.right.span().start as i32 + offset).max(0) as u32,
                    (ap.right.span().end as i32 + offset).max(0) as u32,
                );
                let mut ctx = TemplateCtx {
                    source,
                    scope: parent_scope,
                    lang,
                    in_control_flow: false,
                };
                self.walk_expr_range(abs, &mut ctx, RefFlags::default());
            }
        }
    }

    /// `bind:foo={expr}` behaves like a write to `expr` from
    /// upstream's scope walker (scope.js `BindDirective` pushes to
    /// `updates`). Also captures the bind's BASE identifier (even
    /// when the expression is a member chain like `rest[0]`) onto
    /// the backing binding's `has_bind_reference` flag, for
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
            self.bindings[bid.0 as usize].has_bind_reference = true;
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

    fn build_script(&mut self, script: &ScriptSection<'_>, root_scope: ScopeId) {
        self.build_script_inner(script, root_scope, false, &[]);
    }

    /// Instance-script variant — marks the walker so `$:` labels at
    /// the program top level flip `in_reactive_statement` on descent.
    /// Upstream guards the `reactive_declaration_module_script_dependency`
    /// check behind `ast_type === 'instance'`, so we gate the same way.
    fn build_script_as_instance(
        &mut self,
        script: &ScriptSection<'_>,
        root_scope: ScopeId,
        leading_ignores: &[SmolStr],
    ) {
        self.build_script_inner(script, root_scope, true, leading_ignores);
    }

    fn build_script_inner(
        &mut self,
        script: &ScriptSection<'_>,
        root_scope: ScopeId,
        is_instance: bool,
        leading_ignores: &[SmolStr],
    ) {
        let alloc = oxc_allocator::Allocator::default();
        let parsed = parse_script_body(&alloc, script.content, script.lang);
        let base = script.content_range.start;
        let start_depth = self.scopes[root_scope.0 as usize].function_depth;
        // Pre-collect svelte-ignore comments by source position so
        // the statement visitor can look up leading comments in
        // O(log n) later. Spans are absolute byte offsets into the
        // full .svelte source (not the script-local span).
        let mut script_ignore_comments: Vec<IgnoreSpan> = Vec::new();
        for c in &parsed.program.comments {
            let text = &script.content[c.span.start as usize..c.span.end as usize];
            let Some(body) = strip_comment_delimiters(text) else {
                continue;
            };
            let trimmed = body.trim_start();
            let Some(rest) = trimmed.strip_prefix("svelte-ignore") else {
                continue;
            };
            let rest = match rest.chars().next() {
                Some(ch) if ch.is_whitespace() => &rest[ch.len_utf8()..],
                _ => continue,
            };
            // Runes-mode parsing only matters for our downstream
            // consumer (fires legacy/unknown). The svelte-ignore-
            // comment extraction here just needs to yield the codes
            // that the comment intends to suppress, so use the
            // lenient path.
            let codes = crate::ignore::parse_ignore_codes_public(rest, true);
            // script-local offset — matches stmt.span.start.
            script_ignore_comments.push(IgnoreSpan {
                span_end: c.span.end,
                codes,
            });
        }
        script_ignore_comments.sort_by_key(|c| c.span_end);
        let mut walker = ScriptWalker {
            tree: self,
            base_offset: base,
            scope_stack: vec![root_scope],
            function_depth: start_depth,
            rune_bump: 0,
            in_function_closure: false,
            type_annotation_depth: 0,
            in_state_arg_nested: false,
            in_reactive_statement: false,
            is_instance,
            program_depth: start_depth,
            script_ignore_comments,
            script_content: script.content,
            ignore_frames: Vec::new(),
        };
        // Push the template-comment ignores so they apply to every
        // reference recorded during this script walk.
        if !leading_ignores.is_empty() {
            walker.ignore_frames.push(leading_ignores.to_vec());
        }
        for stmt in &parsed.program.body {
            walker.visit_stmt(stmt);
        }
        if !leading_ignores.is_empty() {
            walker.ignore_frames.pop();
        }
        // oxc types borrow from `alloc`; keep it alive until the walk
        // finishes. Nothing we stash below borrows from it.
        drop(parsed);
        drop(alloc);
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
            custom_element_props_candidates: self.custom_element_props_candidates,
            custom_element_props_ignored: self.custom_element_props_ignored,
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
        let is_rune_name = is_rune_name(n);
        if backing.is_none() && !is_rune_name {
            continue;
        }
        // Upstream guards:
        //   `declaration && get_rune(init) !== null` → DON'T synthesize
        //   EXCEPT the `store_name !== 'props' && get_rune === '$props'`
        //   carve-out (which preserves e.g. `const foo = $props(); $foo()`
        //   as a conflict).
        if let Some(bid) = backing {
            if let InitialKind::RuneCall { rune, .. } = tree.bindings[bid.0 as usize].initial {
                let props_exception = store_name != "props" && rune == RuneCall::Props;
                if !props_exception {
                    continue;
                }
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

/// Pull the leading identifier of a bind-directive expression. For
/// `foo` returns `Some("foo")`; for `foo[0]` returns `Some("foo")`;
/// for `foo.bar.baz` returns `Some("foo")`. Anything else returns
/// `None`.
fn extract_base_ident(s: &str) -> Option<&str> {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i == 0 && !(c.is_ascii_alphabetic() || c == '_' || c == '$') {
            return None;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 { None } else { Some(&s[..end]) }
}

/// Matches upstream `utils.js::is_rune`. Keep in sync with the
/// `RUNES` constant there.
pub fn is_rune_name(name: &str) -> bool {
    matches!(
        name,
        "$state"
            | "$state.raw"
            | "$state.eager"
            | "$state.snapshot"
            | "$derived"
            | "$derived.by"
            | "$props"
            | "$props.id"
            | "$bindable"
            | "$effect"
            | "$effect.pre"
            | "$effect.tracking"
            | "$effect.root"
            | "$effect.pending"
            | "$inspect"
            | "$inspect().with"
            | "$inspect.trace"
            | "$host"
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
    /// When >0, we're inside a TS type annotation subtree — all
    /// identifiers within are types, not value references. Upstream
    /// strips these via `remove_typescript_nodes` before scope walk;
    /// we skip them at walk time instead.
    type_annotation_depth: u32,
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
    /// `function_depth` value at the start of the current script —
    /// used to recognize "top-level of Program" for `$:` statements.
    program_depth: u32,
    /// Pre-collected `// svelte-ignore …` / `/* svelte-ignore … */`
    /// comments in the script body, sorted by `span_end`. Populated
    /// from `parsed.program.comments` before the walk starts.
    /// Offsets are script-local (oxc's span origin = script content
    /// start).
    script_ignore_comments: Vec<IgnoreSpan>,
    /// Script source text (= `ScriptSection::content`) — needed so
    /// `push_leading_ignores` can verify the gap between a comment's
    /// `span_end` and its trailing statement is whitespace-only.
    script_content: &'src str,
    /// Live stack of ignore-code sets — one frame per statement we
    /// entered that had leading `// svelte-ignore` comments. Active
    /// codes at any time = flatten all frames. Snapshot is cloned
    /// onto each `PendingRef` we record.
    ignore_frames: Vec<Vec<SmolStr>>,
}

/// One `// svelte-ignore …` / `/* … */` comment in a script body.
struct IgnoreSpan {
    /// Absolute byte offset (into the source) of the comment's end
    /// — the character immediately after `*/` for block comments
    /// or the `\n` for line comments.
    span_end: u32,
    codes: Vec<SmolStr>,
}

/// Extract the `span.start` of an arbitrary `Statement` — oxc doesn't
/// expose a single uniform `span()` method, so we destructure.
fn statement_span_start(stmt: &Statement<'_>) -> Option<u32> {
    use oxc_span::GetSpan;
    Some(stmt.span().start)
}

/// Peel off TS-only expression wrappers so rune-call detection sees
/// the `$state(…)` call inside `$state<T>() as unknown as X` etc.
/// Mirrors upstream's `remove_typescript_nodes` phase.
fn unwrap_ts_wrappers<'e, 'a>(expr: &'e Expression<'a>) -> &'e Expression<'a> {
    let mut cur = expr;
    loop {
        match cur {
            Expression::TSAsExpression(t) => cur = &t.expression,
            Expression::TSSatisfiesExpression(t) => cur = &t.expression,
            Expression::TSNonNullExpression(t) => cur = &t.expression,
            Expression::TSTypeAssertion(t) => cur = &t.expression,
            Expression::TSInstantiationExpression(t) => cur = &t.expression,
            Expression::ParenthesizedExpression(p) => cur = &p.expression,
            _ => return cur,
        }
    }
}

fn strip_comment_delimiters(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix("//") {
        Some(rest)
    } else if let Some(rest) = text.strip_prefix("/*") {
        Some(rest.trim_end_matches("*/"))
    } else {
        None
    }
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
        let pushed = self.push_leading_ignores(statement_span_start(stmt));
        self.visit_stmt_inner(stmt);
        if pushed {
            self.ignore_frames.pop();
        }
    }

    /// Pull any pre-collected `// svelte-ignore …` comments whose
    /// end-offset immediately precedes `stmt_start` (only whitespace
    /// in between — matches upstream's leadingComments semantic).
    /// Push them as a new ignore frame. Returns `true` if a frame
    /// was pushed so the caller pops on exit.
    fn push_leading_ignores(&mut self, stmt_start: Option<u32>) -> bool {
        let Some(start) = stmt_start else {
            return false;
        };
        let start = start as usize;
        let bytes = self.script_content.as_bytes();
        let mut codes: Vec<SmolStr> = Vec::new();
        // Iterate comments ending at-or-before `start`, most recent
        // first. Stop as soon as we hit a comment whose gap to
        // `start` contains a non-whitespace byte (= not leading).
        for ig in self.script_ignore_comments.iter().rev() {
            let span_end = ig.span_end as usize;
            if span_end > start {
                continue;
            }
            if span_end > bytes.len() || start > bytes.len() {
                break;
            }
            let gap = &bytes[span_end..start];
            if !gap.iter().all(|b| b.is_ascii_whitespace()) {
                break;
            }
            for c in &ig.codes {
                if !codes.contains(c) {
                    codes.push(c.clone());
                }
            }
        }
        if codes.is_empty() {
            false
        } else {
            self.ignore_frames.push(codes);
            true
        }
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

    fn visit_stmt_inner(&mut self, stmt: &Statement<'_>) {
        match stmt {
            Statement::VariableDeclaration(vd) => self.visit_var_decl(vd),
            Statement::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    self.tree.declare(
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
                        self.tree.declare(
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
                                self.tree.declare(
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
                if let Some(init) = &f.init {
                    match init {
                        ForStatementInit::VariableDeclaration(v) => self.visit_var_decl(v),
                        e => {
                            if let Some(expr) = expression_from_for_init(e) {
                                self.visit_expr(expr);
                            }
                        }
                    }
                }
                if let Some(t) = &f.test {
                    self.visit_expr(t);
                }
                if let Some(u) = &f.update {
                    self.visit_expr(u);
                }
                self.visit_stmt(&f.body);
            }
            Statement::ForInStatement(f) => {
                self.visit_expr(&f.right);
                self.visit_stmt(&f.body);
            }
            Statement::ForOfStatement(f) => {
                self.visit_expr(&f.right);
                self.visit_stmt(&f.body);
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
                    for s in &h.body.body {
                        self.visit_stmt(s);
                    }
                }
                if let Some(f) = &t.finalizer {
                    for s in &f.body {
                        self.visit_stmt(s);
                    }
                }
            }
            Statement::SwitchStatement(s) => {
                self.visit_expr(&s.discriminant);
                for case in &s.cases {
                    if let Some(t) = &case.test {
                        self.visit_expr(t);
                    }
                    for s in &case.consequent {
                        self.visit_stmt(s);
                    }
                }
            }
            Statement::ExpressionStatement(es) => self.visit_expr(&es.expression),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.visit_expr(arg);
                }
            }
            Statement::LabeledStatement(lbl) => self.visit_labeled(lbl),
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

    fn visit_labeled(&mut self, lbl: &LabeledStatement<'_>) {
        // `$: …` — upstream puts the LHS name into
        // `possible_implicit_declarations` and promotes it to
        // `legacy_reactive` post-walk if no outer binding exists.
        // Not ported yet. For
        // `reactive_declaration_module_script_dependency` we need to
        // know the reference sits inside a `$:` block at the top
        // level of the instance script.
        let is_top_level_reactive =
            lbl.label.name == "$" && self.is_instance && self.function_depth == self.program_depth;
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
            self.tree.declare(
                self.cur_scope(),
                SmolStr::from(id.name.as_str()),
                self.abs(id.span.start, id.span.end),
                BindingKind::Normal,
                DeclarationKind::Let,
                InitialKind::ClassDecl,
            );
        }
        self.visit_class_body(&cls.body);
    }

    fn visit_class_body(&mut self, body: &ClassBody<'_>) {
        for m in &body.body {
            match m {
                ClassElement::MethodDefinition(md) => {
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
                    if let Some(v) = &p.value {
                        self.visit_expr(v);
                    }
                }
                _ => {}
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
        let is_props_identifier =
            is_props && matches!(&d.id.kind, BindingPatternKind::BindingIdentifier(_));

        // custom_element_props_identifier candidate. Upstream
        // `VariableDeclarator.js:72-83` fires on Identifier form
        // (`let props = $props()` → id span) or ObjectPattern with
        // a rest element (`let { ...props } = $props()` → the
        // RestElement span). Firing is gated downstream by the
        // presence of `<svelte:options customElement={…}>` and the
        // absence of an explicit `props` option on it.
        if is_props {
            let warn_range = match &d.id.kind {
                BindingPatternKind::BindingIdentifier(id) => {
                    Some(self.abs(id.span.start, id.span.end))
                }
                BindingPatternKind::ObjectPattern(op) => {
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
            && let BindingPatternKind::BindingIdentifier(id) = &d.id.kind
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
        match &pat.kind {
            BindingPatternKind::BindingIdentifier(id) => {
                self.tree.declare(
                    self.cur_scope(),
                    SmolStr::from(id.name.as_str()),
                    self.abs(id.span.start, id.span.end),
                    kind,
                    decl_kind,
                    initial.clone(),
                );
            }
            BindingPatternKind::ObjectPattern(op) => {
                self.declare_object_pattern(op, decl_kind, kind, initial, is_props);
            }
            BindingPatternKind::ArrayPattern(ap) => {
                self.declare_array_pattern(ap, decl_kind, kind, initial, is_props);
            }
            BindingPatternKind::AssignmentPattern(ap) => {
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
                    let default =
                        if let BindingPatternKind::AssignmentPattern(ap) = &prop.value.kind {
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
        match &pat.kind {
            BindingPatternKind::AssignmentPattern(ap) => {
                self.visit_expr(&ap.right);
                self.walk_pattern_defaults(&ap.left);
            }
            BindingPatternKind::ObjectPattern(op) => {
                for prop in &op.properties {
                    self.walk_pattern_defaults(&prop.value);
                }
                if let Some(rest) = &op.rest {
                    self.walk_pattern_defaults(&rest.argument);
                }
            }
            BindingPatternKind::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    self.walk_pattern_defaults(p);
                }
                if let Some(rest) = &ap.rest {
                    self.walk_pattern_defaults(&rest.argument);
                }
            }
            BindingPatternKind::BindingIdentifier(_) => {}
        }
    }

    fn visit_expr(&mut self, e: &Expression<'_>) {
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
                    if let Some(e) = a.as_expression() {
                        self.visit_expr(e);
                    }
                }
            }
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
                        AE::SpreadElement(s) => self.visit_expr(&s.argument),
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
                // `(/* svelte-ignore CODE */ expr)` — honor per-node
                // svelte-ignore comments attached to the inner
                // expression. Statement-level leading-comment
                // capture doesn't see these because the comment
                // lives inside the parens, not before the statement.
                // Mirrors upstream's per-node `ignore_map` model.
                let pushed = self.push_leading_ignores(Some(p.expression.span().start));
                self.visit_expr(&p.expression);
                if pushed {
                    self.ignore_frames.pop();
                }
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
            Expression::AwaitExpression(a) => self.visit_expr(&a.argument),
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
            if let Some(e) = a.as_expression() {
                // Honour `// svelte-ignore CODE` comments that precede
                // this argument — upstream attaches leading comments
                // per-node, so a runed-rune call with the ignore
                // between `(` and the expression silences a rule for
                // references nested inside. Statement-level capture
                // (which `visit_stmt` does) isn't enough here because
                // the comment lives *inside* the surrounding
                // declaration statement, not before it.
                let pushed = self.push_leading_ignores(Some(e.span().start));
                if push_state {
                    self.visit_arg_inside_state_call(e);
                } else {
                    self.visit_expr(e);
                }
                if pushed {
                    self.ignore_frames.pop();
                }
            }
        }
        if bump {
            self.rune_bump -= 1;
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
            AssignmentTarget::ArrayAssignmentTarget(_)
            | AssignmentTarget::ObjectAssignmentTarget(_) => {
                // Destructuring assignment — upstream `unwrap_pattern`
                // would extract each leaf; we skip for now since the
                // 4 target rules don't need it.
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
        if self.type_annotation_depth > 0 {
            return;
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

/// For a `$state`/`$state.raw` call init, return whether the first
/// argument is a primitive-like (matching upstream's `should_proxy`
/// analog). `true` if no argument.
fn state_rune_primitive_arg(e: &Expression<'_>) -> bool {
    if let Expression::CallExpression(c) = e {
        c.arguments
            .first()
            .and_then(|a| a.as_expression())
            .map(is_primitive_expr)
            .unwrap_or(true)
    } else {
        true
    }
}

fn detect_rune_call_from_call(c: &CallExpression<'_>) -> Option<RuneCall> {
    let callee_name = match &c.callee {
        Expression::Identifier(id) => id.name.as_str().to_string(),
        Expression::StaticMemberExpression(m) => {
            if let Expression::Identifier(o) = &m.object {
                format!("{}.{}", o.name.as_str(), m.property.name.as_str())
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(match callee_name.as_str() {
        "$state" => RuneCall::State,
        "$state.raw" => RuneCall::StateRaw,
        "$derived" => RuneCall::Derived,
        "$derived.by" => RuneCall::DerivedBy,
        "$props" => RuneCall::Props,
        "$bindable" => RuneCall::Bindable,
        "$inspect" => RuneCall::Inspect,
        "$host" => RuneCall::Host,
        "$effect" => RuneCall::Effect,
        _ => return None,
    })
}

/// Detects `$bindable(default)` inside a $props() destructure default
/// position. Returns `Some(primitive)` where primitive is whether the
/// arg is a primitive-literal-ish thing, or `None` if not a $bindable
/// call.
fn detect_bindable_default(pat: &BindingPattern<'_>) -> Option<bool> {
    match &pat.kind {
        BindingPatternKind::AssignmentPattern(ap) => match &ap.right {
            Expression::CallExpression(c) => {
                if detect_rune_call_from_call(c) == Some(RuneCall::Bindable) {
                    let arg_is_primitive = c
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(is_primitive_expr)
                        .unwrap_or(true);
                    Some(arg_is_primitive)
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Conservative `should_proxy`-analog — upstream
/// `3-transform/client/utils.js::should_proxy`. Returns `true` if the
/// expression is one of the primitive-like kinds that should NOT be
/// proxied.
fn is_primitive_expr(e: &Expression<'_>) -> bool {
    matches!(
        e,
        Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::UnaryExpression(_)
            | Expression::BinaryExpression(_)
    ) || matches!(e, Expression::Identifier(id) if id.name.as_str() == "undefined")
}

fn base_identifier<'a>(e: &'a Expression<'_>) -> Option<(&'a str, u32, u32)> {
    match e {
        Expression::Identifier(id) => Some((id.name.as_str(), id.span.start, id.span.end)),
        Expression::StaticMemberExpression(m) => base_identifier(&m.object),
        Expression::ComputedMemberExpression(m) => base_identifier(&m.object),
        _ => None,
    }
}

fn expression_from_for_init<'a>(e: &'a ForStatementInit<'_>) -> Option<&'a Expression<'a>> {
    e.as_expression()
}

fn expression_from_default<'a>(
    e: &'a oxc_ast::ast::ExportDefaultDeclarationKind<'_>,
) -> Option<&'a Expression<'a>> {
    e.as_expression()
}

fn expression_from_property_key<'a>(k: &'a PropertyKey<'_>) -> Option<&'a Expression<'a>> {
    k.as_expression()
}
