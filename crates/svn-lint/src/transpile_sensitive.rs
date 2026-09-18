//! Whether a component's compiler diagnostics depend on what
//! TypeScript's transpile really prints.
//!
//! Without a Svelte config, the language server preprocesses every
//! `<script lang="ts">` with `ts.transpileModule` and compiles the
//! output, mapping positions back through the preprocessor's source
//! map. The lint pass normally reads the original TypeScript and
//! models the transpile in place: type syntax disappears, everything
//! else keeps its position. That model breaks down where TypeScript
//! prints something structurally different from the input:
//!
//! - a script TypeScript cannot parse: its error recovery decides the
//!   printed code (`let class = 1` becomes `let; class {} 1;`);
//! - constructs TypeScript rewrites into other code: enums and
//!   namespaces become IIFEs (a class inside one becomes a nested
//!   class), parameter properties become class fields, `import x =`
//!   and `export =` become plain statements;
//! - constructs the compiler rejects with a position taken from the
//!   printed layout (`accessor` fields, decorators).
//!
//! A syntax error in a script the preprocessor leaves alone matters
//! too once some other script is transpiled: the error's position
//! then goes through the whole-component source map, which maps a
//! position inside a word to the word's start.
//!
//! Components with any of these are compiled from TypeScript's real
//! output instead (by the CLI, which owns the compiler process). That
//! output comes from tsgo, whose printer differs from TypeScript 5's
//! (which svelte-check runs) in a few places; a script that has one of
//! those keeps the model (see [`printed_differently_by_tsgo`]).

use oxc_ast::ast::{
    AccessorProperty, Decorator, Expression, FormalParameter, TSEnumDeclaration,
    TSExportAssignment, TSImportEqualsDeclaration, TSNamespaceDeclaration,
    TaggedTemplateExpression,
};
use oxc_ast_visit::{Visit, walk};
use svn_parser::{ParsedScript, ScriptSection};

use crate::rules::transpile_positions::namespace_removed;
use crate::rules::typescript_features::script_is_transpiled;

/// Whether the language server's fallback preprocessor output decides
/// this component's compiler diagnostics (see the module docs). Each
/// script comes with its parse.
pub(crate) fn needs_real_transpile(scripts: &[(&ScriptSection<'_>, &ParsedScript<'_>)]) -> bool {
    if !scripts
        .iter()
        .any(|(section, _)| script_is_transpiled(section, true))
    {
        return false;
    }
    scripts.iter().any(|(section, parsed)| {
        if parsed.panicked || !parsed.errors.is_empty() {
            return true;
        }
        if !script_is_transpiled(section, true) {
            return false;
        }
        let mut finder = RewriteFinder { found: false };
        finder.visit_program(&parsed.program);
        finder.found
    })
}

/// Finds the first construct TypeScript prints as different code.
struct RewriteFinder {
    found: bool,
}

impl<'a> Visit<'a> for RewriteFinder {
    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        if !it.declare {
            self.found = true;
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        if !namespace_removed(it) {
            self.found = true;
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if !it.import_kind.is_type() {
            self.found = true;
        }
    }

    fn visit_ts_export_assignment(&mut self, _it: &TSExportAssignment<'a>) {
        self.found = true;
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.accessibility.is_some() || it.readonly || it.r#override {
            self.found = true;
        } else {
            walk::walk_formal_parameter(self, it);
        }
    }

    fn visit_accessor_property(&mut self, _it: &AccessorProperty<'a>) {
        self.found = true;
    }

    fn visit_decorator(&mut self, _it: &Decorator<'a>) {
        self.found = true;
    }
}

/// Whether tsgo prints one of the scripts differently from TypeScript 5,
/// so its output cannot stand in for the preprocessor's. tsgo wraps an
/// optional chain used as a template tag in parentheses (`a?.b`x``
/// prints as `(a?.b) `x``), which turns the compiler's syntax error
/// into valid code.
pub(crate) fn printed_differently_by_tsgo(
    scripts: &[(&ScriptSection<'_>, &ParsedScript<'_>)],
) -> bool {
    scripts.iter().any(|(section, parsed)| {
        if !script_is_transpiled(section, true) {
            return false;
        }
        let mut finder = PrinterDifferenceFinder { found: false };
        finder.visit_program(&parsed.program);
        finder.found
    })
}

struct PrinterDifferenceFinder {
    found: bool,
}

impl<'a> Visit<'a> for PrinterDifferenceFinder {
    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        if tag_has_optional_chain(&it.tag) {
            self.found = true;
        }
        walk::walk_tagged_template_expression(self, it);
    }
}

/// Whether a tagged template's tag is (or ends in) an optional chain.
/// A well-formed parse wraps it in a chain expression; when the parser
/// has already rejected the construct it keeps the bare member or call
/// with its `optional` flag instead.
pub(crate) fn tag_has_optional_chain(tag: &Expression<'_>) -> bool {
    match tag {
        Expression::ChainExpression(_) => true,
        Expression::StaticMemberExpression(m) => m.optional || tag_has_optional_chain(&m.object),
        Expression::ComputedMemberExpression(m) => m.optional || tag_has_optional_chain(&m.object),
        Expression::PrivateFieldExpression(m) => m.optional || tag_has_optional_chain(&m.object),
        Expression::CallExpression(c) => c.optional || tag_has_optional_chain(&c.callee),
        _ => false,
    }
}
