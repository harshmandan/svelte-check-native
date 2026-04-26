//! TypeScript-AST traversal helpers used by the script-wrapping
//! layer.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/tsAst.ts`.
//!
//! **Status: NA — we use oxc directly.**
//!
//! Upstream's `tsAst.ts` exports helpers like `findDefaultExport`,
//! `getDeclaratorName`, generic AST-walk wrappers around the
//! TypeScript Compiler API. They wrap TS's `node.kind` discriminator
//! checks in friendlier names so the rest of the codebase can ask
//! "is this a default export?" without typing
//! `node.modifiers?.some(m => m.kind === SyntaxKind.DefaultKeyword)`.
//!
//! Our equivalent is direct oxc usage. `oxc_ast::ast::Statement`,
//! `oxc_ast::ast::Expression`, etc. are pattern-matchable Rust enums —
//! checking for a default export is `matches!(stmt,
//! Statement::ExportDefaultDeclaration(_))` inline at each call site.
//! Rust's enum exhaustiveness + pattern syntax makes the wrapper
//! layer unnecessary.
//!
//! Concrete cross-references (where contributors would land if
//! grepping upstream symbol names):
//!
//! | Upstream function | Our equivalent |
//! |---|---|
//! | `findDefaultExport(program)` | inline `program.body.iter().find_map(|s| matches!(s, Statement::ExportDefaultDeclaration(_)))` at each call site (used in `inline_component` and `props_emit`). |
//! | `getDeclaratorName(decl)` | inline `BindingPatternKind::BindingIdentifier(id) => id.name.as_str()` pattern-match at each call site. |
//! | `walk(node, visitor)` | oxc's visitor traits (`oxc_ast::Visit`) — we implement these per-walker rather than passing a callback. |
//! | `isInterfaceDeclaration(node)` etc. | `matches!(stmt, Statement::TSInterfaceDeclaration(_))` and similar. |
//!
//! This file is a navigational stub only.
