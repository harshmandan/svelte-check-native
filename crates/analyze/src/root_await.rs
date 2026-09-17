//! Does the instance script `await` at its root scope?
//!
//! Mirrors upstream `processInstanceScriptContent.ts`'s
//! `hasTopLevelAwait`: it pushes a scope for every block and every
//! function, and counts an `await` only when the current scope is the
//! root one. So `const x = await f()` at the top of the script counts,
//! but an `await` inside a `try`, an `if` body or a bare `{ … }` block
//! does not — and then the render function stays synchronous, so
//! TypeScript reports TS1308 on that `await`, as upstream does.

use oxc_ast::ast::{ArrowFunctionExpression, AwaitExpression, BlockStatement, Function, Program};
use oxc_ast_visit::{Visit, walk};

pub fn has_root_scope_await(program: &Program<'_>) -> bool {
    let mut probe = RootAwaitProbe {
        depth: 0,
        found: false,
    };
    probe.visit_program(program);
    probe.found
}

struct RootAwaitProbe {
    depth: u32,
    found: bool,
}

impl<'a> Visit<'a> for RootAwaitProbe {
    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        if self.depth == 0 {
            self.found = true;
        }
        walk::walk_await_expression(self, it);
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.depth += 1;
        walk::walk_block_statement(self, it);
        self.depth -= 1;
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.depth += 1;
        walk::walk_function(self, it, flags);
        self.depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.depth += 1;
        walk::walk_arrow_function_expression(self, it);
        self.depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::has_root_scope_await;

    fn probe(src: &str) -> bool {
        let alloc = oxc_allocator::Allocator::default();
        let parsed = svn_parser::parse_script_body(&alloc, src, svn_parser::ScriptLang::Ts);
        has_root_scope_await(&parsed.program)
    }

    #[test]
    fn root_await_counts_nested_does_not() {
        assert!(probe("const x = await f();"));
        assert!(probe("if (await f()) {}"));
        assert!(probe("if (a) data = await f();"));
        assert!(!probe("try { data = await f(); } catch {}"));
        assert!(!probe("{ data = await f(); }"));
        assert!(!probe("if (a) { const m = await f(); }"));
        assert!(!probe("async function g() { await f(); }"));
        assert!(!probe("const g = async () => { await f(); };"));
    }
}
