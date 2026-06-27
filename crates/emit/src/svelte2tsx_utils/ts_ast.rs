//! TypeScript-AST traversal helpers used by the script-wrapping
//! layer.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/tsAst.ts`.
//!
//! **Status: NA — we use oxc directly.**
//!
//! Upstream's `tsAst.ts` exports helpers like
//! `isInterfaceOrTypeDeclaration`, `findExportKeyword`,
//! `getVariableAtTopLevel`, and `getTopLevelImports` — thin wrappers
//! around the TypeScript Compiler API. They wrap TS's `node.kind`
//! discriminator checks in friendlier names so the rest of the
//! codebase can ask "is this an interface or type alias?" without
//! typing `node.kind === SyntaxKind.InterfaceDeclaration || node.kind
//! === SyntaxKind.TypeAliasDeclaration`.
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
//! | `isInterfaceOrTypeDeclaration(node)` | `matches!(stmt, Statement::TSInterfaceDeclaration(_) \| Statement::TSTypeAliasDeclaration(_))` |
//! | `findExportKeyword(node)` | match on `Statement::Export*` / `decl.declare` at the call site |
//! | `getVariableAtTopLevel(sf, name)` | `program.body.iter().find_map(...)` over top-level `VariableDeclaration`s |
//! | `getTopLevelImports(sf)` | `program.body.iter().filter(\|s\| matches!(s, Statement::ImportDeclaration(_)))` |
//!
//! This file is a navigational stub only.
