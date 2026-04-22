//! Rules that fire on `<Component>` invocations.

use svn_parser::ast::Component;

use crate::context::LintContext;
use crate::rules::element_rules::{AttrParent, visit_attribute};

pub fn visit(comp: &Component, ctx: &mut LintContext<'_>) {
    for attr in &comp.attributes {
        visit_attribute(attr, ctx, AttrParent::Component);
    }
}
