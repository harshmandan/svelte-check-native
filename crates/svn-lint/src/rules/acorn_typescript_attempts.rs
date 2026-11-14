//! The error acorn-typescript settles on for an expression that starts
//! with `<`.
//!
//! Where an assignment-level expression starts with `<`, acorn-typescript
//! does not know whether it reads an arrow function with type parameters
//! (`<T>(a) => a`) or a type assertion (`<T>a`). Its `parseMaybeAssign`
//! tries both:
//!
//! 1. The arrow: type parameters, then an expression that has to be an
//!    arrow function.
//! 2. Otherwise the expression as it stands. There `<T>` opens a type
//!    assertion, and when the assertion cannot be read (the type or its
//!    operand fails), the `<…>` is read as type parameters once more and
//!    returned as if it were the whole expression.
//!
//! When both attempts throw, the arrow's error is the one reported. When
//! the second "succeeds" by falling back to type parameters, the parser
//! carries on at the token after the `>`, which nothing allows there.
//!
//! So an error inside an arrow function with type parameters is not
//! reported where it lies: `<T>(a) => { let = 1 }` is an unexpected `=>`,
//! because `<T>(a)` reads as an assertion on its own. And `<let>y + ;` is
//! a reserved word, because a type parameter's name is a binding.
//!
//! oxc reads the same text one way only. This takes the error found on
//! that reading and the tree it was found in, and gives the error
//! acorn-typescript ends up with.

use oxc_ast::ast::{
    ArrowFunctionExpression, AwaitExpression, BinaryExpression, Expression, LogicalExpression,
    Program, TSTypeAssertion, UnaryExpression,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;

use crate::rules::acorn_early_errors::{Candidate, Mode, Position, next_token_start};
use crate::rules::js_parse_error::first_error_in;

/// What oxc read at the `<` an expression starts with.
#[derive(Clone, Copy)]
enum Reading {
    Unknown,
    /// `<type>operand`.
    Assertion {
        type_start: u32,
        type_end: u32,
        operand_start: u32,
        operand_end: u32,
    },
    /// `<type parameters>(parameters) => body`.
    Arrow {
        /// The type parameters are one plain name, which reads as the
        /// type of an assertion too.
        one_plain_type: bool,
        /// The parameters read as a parenthesized expression: there is
        /// one, and none is a rest element.
        parameters_are_an_operand: bool,
        parameters_start: u32,
        parameters_end: u32,
    },
}

/// An expression that starts with `<` where acorn-typescript calls
/// `parseMaybeAssign`.
struct Site {
    start: u32,
    end: u32,
    reading: Reading,
}

struct Sites<'t> {
    text: &'t str,
    /// Starts of operands read without `parseMaybeAssign`: the right
    /// side of a binary operator, the operand of a unary one.
    operands: Vec<u32>,
    found: Vec<Site>,
}

impl Sites<'_> {
    fn site_mut(&mut self, start: u32) -> Option<&mut Site> {
        self.found.iter_mut().find(|site| site.start == start)
    }
}

impl<'a> Visit<'a> for Sites<'_> {
    fn visit_expression(&mut self, it: &Expression<'a>) {
        let span = it.span();
        let starts_with_angle = self.text.as_bytes().get(span.start as usize) == Some(&b'<');
        if starts_with_angle
            && !self.operands.contains(&span.start)
            && !matches!(it, Expression::SequenceExpression(_))
        {
            // The outermost expression starting here is the one
            // `parseMaybeAssign` reads; the others are its left sides.
            match self.site_mut(span.start) {
                Some(site) => site.end = site.end.max(span.end),
                None => self.found.push(Site {
                    start: span.start,
                    end: span.end,
                    reading: Reading::Unknown,
                }),
            }
        }
        walk::walk_expression(self, it);
    }

    fn visit_binary_expression(&mut self, it: &BinaryExpression<'a>) {
        self.operands.push(it.right.span().start);
        walk::walk_binary_expression(self, it);
    }

    fn visit_logical_expression(&mut self, it: &LogicalExpression<'a>) {
        self.operands.push(it.right.span().start);
        walk::walk_logical_expression(self, it);
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        self.operands.push(it.argument.span().start);
        walk::walk_unary_expression(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.operands.push(it.argument.span().start);
        walk::walk_await_expression(self, it);
    }

    fn visit_ts_type_assertion(&mut self, it: &TSTypeAssertion<'a>) {
        let type_span = it.type_annotation.span();
        let operand = it.expression.span();
        if let Some(site) = self.site_mut(it.span.start) {
            site.reading = Reading::Assertion {
                type_start: type_span.start,
                type_end: type_span.end,
                operand_start: operand.start,
                operand_end: operand.end,
            };
        }
        self.operands.push(operand.start);
        walk::walk_ts_type_assertion(self, it);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if let Some(type_parameters) = &it.type_parameters {
            let one_plain_type = match type_parameters.params.as_slice() {
                [only] => {
                    only.constraint.is_none()
                        && only.default.is_none()
                        && !only.r#in
                        && !only.out
                        && !only.r#const
                        && self
                            .text
                            .get(only.span.end as usize..type_parameters.span.end as usize)
                            .is_some_and(|rest| rest.trim() == ">")
                }
                _ => false,
            };
            let reading = Reading::Arrow {
                one_plain_type,
                parameters_are_an_operand: parameters_are_an_operand(self.text, it),
                parameters_start: it.params.span.start,
                parameters_end: it.params.span.end,
            };
            if let Some(site) = self.site_mut(it.span.start) {
                site.reading = reading;
            }
        }
        walk::walk_arrow_function_expression(self, it);
    }
}

/// The error acorn-typescript reports where `found` is the error on
/// oxc's reading of `text` and `program` the tree of that reading (of
/// the text completed past the error, when oxc gave up on it).
///
/// `read_to` is how far `program` follows `text`: all of it, or up to
/// where oxc gave up.
pub(crate) fn settled_error(
    text: &str,
    program: &Program<'_>,
    read_to: u32,
    mode: Mode,
    found: Candidate,
) -> Candidate {
    if !text.contains('<') {
        return found;
    }
    let mut sites = Sites {
        text,
        operands: Vec::new(),
        found: Vec::new(),
    };
    sites.visit_program(program);
    // Each expression the error lies in has made its two attempts by
    // the time the error leaves it, the innermost first.
    let at = found.detect;
    let mut around: Vec<&Site> = sites
        .found
        .iter()
        .filter(|site| site.start < at && at < site.end)
        .collect();
    around.sort_by_key(|site| std::cmp::Reverse(site.start));

    let unexpected = |at: u32| Candidate::at(at, "Unexpected token");
    let mut settled = found;
    for site in around {
        match site.reading {
            Reading::Unknown => {}
            Reading::Assertion {
                type_start,
                type_end,
                operand_start,
                operand_end,
            } => {
                if at < type_end || operand_start >= read_to {
                    continue;
                }
                // Both attempts start with the type parameters, and
                // both throw where those cannot be read.
                if let Some(error) = type_parameters_error(text, type_start, type_end, mode) {
                    settled = error;
                } else if at < operand_end {
                    // The assertion cannot be read, the type parameters
                    // can: the parser carries on at the operand.
                    settled = unexpected(operand_start);
                }
            }
            Reading::Arrow {
                one_plain_type,
                parameters_are_an_operand,
                parameters_start,
                parameters_end,
            } => {
                if parameters_start >= read_to {
                    continue;
                }
                // `<T>(a)` is an assertion by itself when `T` reads as
                // a type, `(a)` as an operand, and the error is not in
                // there.
                let assertion_reads = one_plain_type
                    && parameters_are_an_operand
                    && parameters_end <= read_to
                    && !(parameters_start <= at && at < parameters_end);
                if at < parameters_start && !assertion_reads {
                    // Neither attempt gets past the type parameters.
                    continue;
                }
                settled = if assertion_reads {
                    // The return type or the `=>` follows the assertion.
                    unexpected(next_token_start(text, parameters_end))
                } else {
                    // The type parameters stand alone, and the
                    // parameters follow them.
                    unexpected(parameters_start)
                };
            }
        }
    }
    settled
}

/// The error of reading `text[start..end]`, the type of an assertion,
/// as a list of type parameters.
fn type_parameters_error(text: &str, start: u32, end: u32, mode: Mode) -> Option<Candidate> {
    const BEFORE: &str = "function _<";
    let list = text.get(start as usize..end as usize)?;
    let wrapped = format!("{BEFORE}{list}>() {{}}");
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let parsed = oxc_parser::Parser::new(&allocator, &wrapped, source_type).parse();
    let error = first_error_in(
        &wrapped,
        &parsed.program,
        &parsed.diagnostics,
        parsed.fatal_error,
        mode,
    )?;
    let Position::At(offset) = error.at else {
        return None;
    };
    let inside = offset.checked_sub(BEFORE.len() as u32)?;
    (inside < end - start).then(|| Candidate::at(start + inside, error.message))
}

/// Whether an arrow function's parameters read as a parenthesized
/// expression, the operand of a type assertion: there is one (a `this`
/// parameter counts), none is a rest element, and no comma trails the
/// last.
fn parameters_are_an_operand(text: &str, arrow: &ArrowFunctionExpression<'_>) -> bool {
    let parameters = &arrow.params;
    let Some(list) = text
        .get(parameters.span.start as usize..parameters.span.end as usize)
        .and_then(|list| list.strip_prefix('('))
        .and_then(|list| list.strip_suffix(')'))
        .map(str::trim)
    else {
        return false;
    };
    !list.is_empty() && !list.ends_with(',') && parameters.rest.is_none()
}
