//! Port of TypeScript's `Version` / `VersionRange` semver subset
//! (`compiler/semver.ts`), used to evaluate versioned `types@<range>`
//! condition keys in package.json `exports` maps — the check TS calls
//! `isApplicableVersionedTypesKey`.
//!
//! Grammar (TS `parseRange`): `||`-separated alternatives; each is either
//! a hyphen range `A - B` or whitespace-separated comparators
//! `(~|^|<|<=|>|>=|=)? partial`, where a partial is
//! `major[.minor[.patch[-prerelease][+build]]]` with `x` / `X` / `*`
//! wildcards. An empty range matches every version. `test` is
//! prerelease-inclusive, equivalent to node-semver's
//! `satisfies(version, range, { includePrerelease: true })`.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<Identifier>,
}

/// One dot-separated prerelease identifier. Numeric identifiers compare
/// by value and rank below every alphanumeric one; alphanumeric compare
/// ordinally — semver precedence, as TS's `comparePrereleaseIdentifiers`
/// implements it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identifier {
    Numeric(u64),
    Alpha(String),
}

impl Version {
    pub(crate) const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            prerelease: Vec::new(),
        }
    }

    fn with_prerelease_zero(&self) -> Self {
        Self {
            prerelease: vec![Identifier::Numeric(0)],
            ..self.clone()
        }
    }

    fn increment_major(&self) -> Self {
        Self::new(self.major + 1, 0, 0)
    }

    fn increment_minor(&self) -> Self {
        Self::new(self.major, self.minor + 1, 0)
    }

    fn increment_patch(&self) -> Self {
        Self::new(self.major, self.minor, self.patch + 1)
    }

    /// TS `Version.zero` is `0.0.0-0` — the floor below every real
    /// version, so `< zero` is a comparator nothing satisfies.
    fn zero() -> Self {
        Version::new(0, 0, 0).with_prerelease_zero()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| compare_prerelease(&self.prerelease, &other.prerelease))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver prerelease precedence: a release outranks its prereleases;
/// otherwise identifiers compare pairwise, and when one list is a
/// prefix of the other, the longer list ranks higher.
fn compare_prerelease(left: &[Identifier], right: &[Identifier]) -> Ordering {
    if left.is_empty() {
        return if right.is_empty() {
            Ordering::Equal
        } else {
            Ordering::Greater
        };
    }
    if right.is_empty() {
        return Ordering::Less;
    }
    for (l, r) in left.iter().zip(right.iter()) {
        let ord = match (l, r) {
            (Identifier::Numeric(a), Identifier::Numeric(b)) => a.cmp(b),
            (Identifier::Numeric(_), Identifier::Alpha(_)) => Ordering::Less,
            (Identifier::Alpha(_), Identifier::Numeric(_)) => Ordering::Greater,
            (Identifier::Alpha(a), Identifier::Alpha(b)) => a.cmp(b),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    left.len().cmp(&right.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

#[derive(Debug, Clone)]
struct Comparator {
    op: Op,
    operand: Version,
}

fn cmp(op: Op, operand: Version) -> Comparator {
    Comparator { op, operand }
}

/// A parsed partial version: the concrete `Version` (wildcard fields
/// zeroed, as TS constructs it) plus which fields were wildcards.
struct Partial {
    version: Version,
    major_wild: bool,
    minor_wild: bool,
    patch_wild: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct VersionRange {
    /// Disjunction of conjunctions: the range matches when any
    /// alternative's comparators all hold. No alternatives at all (an
    /// empty range) matches everything.
    alternatives: Vec<Vec<Comparator>>,
}

impl VersionRange {
    pub(crate) fn try_parse(text: &str) -> Option<Self> {
        let mut alternatives = Vec::new();
        for range in text.trim().split("||") {
            // TS skips empty pieces before trimming (`a||` has a valid
            // empty tail) but rejects whitespace-only pieces after
            // (`a || ` splits to a piece that trims to nothing and
            // fails the comparator grammar).
            if range.is_empty() {
                continue;
            }
            let range = range.trim();
            if range.is_empty() {
                return None;
            }
            let mut comparators = Vec::new();
            let tokens: Vec<&str> = range.split_whitespace().collect();
            if tokens.len() == 3 && tokens[1] == "-" {
                if !parse_hyphen(tokens[0], tokens[2], &mut comparators) {
                    return None;
                }
            } else {
                for simple in tokens {
                    if !parse_comparator_token(simple, &mut comparators) {
                        return None;
                    }
                }
            }
            alternatives.push(comparators);
        }
        Some(Self { alternatives })
    }

    pub(crate) fn test(&self, version: &Version) -> bool {
        if self.alternatives.is_empty() {
            return true;
        }
        self.alternatives
            .iter()
            .any(|alt| alt.iter().all(|c| test_comparator(version, c)))
    }
}

fn test_comparator(version: &Version, comparator: &Comparator) -> bool {
    let ord = version.cmp(&comparator.operand);
    match comparator.op {
        Op::Lt => ord == Ordering::Less,
        Op::Le => ord != Ordering::Greater,
        Op::Gt => ord == Ordering::Greater,
        Op::Ge => ord != Ordering::Less,
        Op::Eq => ord == Ordering::Equal,
    }
}

/// TS `parseHyphen`: `A - B` becomes `>= A` (unless A is `*`) and an
/// upper bound derived from how much of B is concrete.
fn parse_hyphen(left: &str, right: &str, out: &mut Vec<Comparator>) -> bool {
    let Some(l) = parse_partial(left) else {
        return false;
    };
    let Some(r) = parse_partial(right) else {
        return false;
    };
    if !l.major_wild {
        out.push(cmp(Op::Ge, l.version));
    }
    if !r.major_wild {
        out.push(if r.minor_wild {
            cmp(Op::Lt, r.version.increment_major())
        } else if r.patch_wild {
            cmp(Op::Lt, r.version.increment_minor())
        } else {
            cmp(Op::Le, r.version)
        });
    }
    true
}

fn parse_comparator_token(token: &str, out: &mut Vec<Comparator>) -> bool {
    let (op, text) = if let Some(rest) = token.strip_prefix("<=") {
        (Some("<="), rest)
    } else if let Some(rest) = token.strip_prefix(">=") {
        (Some(">="), rest)
    } else if let Some(rest) = token.strip_prefix('~') {
        (Some("~"), rest)
    } else if let Some(rest) = token.strip_prefix('^') {
        (Some("^"), rest)
    } else if let Some(rest) = token.strip_prefix('<') {
        (Some("<"), rest)
    } else if let Some(rest) = token.strip_prefix('>') {
        (Some(">"), rest)
    } else if let Some(rest) = token.strip_prefix('=') {
        (Some("="), rest)
    } else {
        (None, token)
    };
    parse_comparator(op, text, out)
}

/// TS `parseComparator`, branch for branch. A wildcard major matches
/// everything except under `<` / `>`, which become unsatisfiable.
fn parse_comparator(op: Option<&str>, text: &str, out: &mut Vec<Comparator>) -> bool {
    let Some(p) = parse_partial(text) else {
        return false;
    };
    if !p.major_wild {
        match op {
            Some("~") => {
                out.push(cmp(Op::Ge, p.version.clone()));
                out.push(cmp(
                    Op::Lt,
                    if p.minor_wild {
                        p.version.increment_major()
                    } else {
                        p.version.increment_minor()
                    },
                ));
            }
            Some("^") => {
                out.push(cmp(Op::Ge, p.version.clone()));
                out.push(cmp(
                    Op::Lt,
                    if p.version.major > 0 || p.minor_wild {
                        p.version.increment_major()
                    } else if p.version.minor > 0 || p.patch_wild {
                        p.version.increment_minor()
                    } else {
                        p.version.increment_patch()
                    },
                ));
            }
            Some(o @ ("<" | ">=")) => {
                let operand = if p.minor_wild || p.patch_wild {
                    p.version.with_prerelease_zero()
                } else {
                    p.version
                };
                out.push(cmp(if o == "<" { Op::Lt } else { Op::Ge }, operand));
            }
            Some(o @ ("<=" | ">")) => {
                out.push(if p.minor_wild {
                    cmp(
                        if o == "<=" { Op::Lt } else { Op::Ge },
                        p.version.increment_major().with_prerelease_zero(),
                    )
                } else if p.patch_wild {
                    cmp(
                        if o == "<=" { Op::Lt } else { Op::Ge },
                        p.version.increment_minor().with_prerelease_zero(),
                    )
                } else {
                    cmp(if o == "<=" { Op::Le } else { Op::Gt }, p.version)
                });
            }
            Some("=") | None => {
                if p.minor_wild || p.patch_wild {
                    out.push(cmp(Op::Ge, p.version.with_prerelease_zero()));
                    out.push(cmp(
                        Op::Lt,
                        if p.minor_wild {
                            p.version.increment_major().with_prerelease_zero()
                        } else {
                            p.version.increment_minor().with_prerelease_zero()
                        },
                    ));
                } else {
                    out.push(cmp(Op::Eq, p.version));
                }
            }
            _ => return false,
        }
    } else if matches!(op, Some("<") | Some(">")) {
        out.push(cmp(Op::Lt, Version::zero()));
    }
    true
}

/// TS `parsePartial` / `partialRegExp`:
/// `^([x*0]|[1-9]\d*)(?:\.([x*0]|[1-9]\d*)(?:\.([x*0]|[1-9]\d*)
/// (?:-([a-z0-9-.]+))?(?:\+([a-z0-9-.]+))?)?)?$` (case-insensitive) —
/// so prerelease/build attach only to a full `major.minor.patch`, and
/// omitted fields default to wildcard.
fn parse_partial(text: &str) -> Option<Partial> {
    let (rest, build) = match text.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (text, None),
    };
    if let Some(b) = build
        && (b.is_empty() || !is_pre_charset(b))
    {
        return None;
    }
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    if let Some(p) = pre
        && (p.is_empty() || !is_pre_charset(p))
    {
        return None;
    }
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    if (pre.is_some() || build.is_some()) && parts.len() != 3 {
        return None;
    }
    let (major, major_wild) = parse_part(parts[0])?;
    let (minor, minor_wild) = match parts.get(1) {
        Some(p) => parse_part(p)?,
        None => (0, true),
    };
    let (patch, patch_wild) = match parts.get(2) {
        Some(p) => parse_part(p)?,
        None => (0, true),
    };
    let prerelease = match pre {
        Some(p) => parse_prerelease(p)?,
        None => Vec::new(),
    };
    Some(Partial {
        version: Version {
            major: if major_wild { 0 } else { major },
            minor: if major_wild || minor_wild { 0 } else { minor },
            patch: if major_wild || minor_wild || patch_wild {
                0
            } else {
                patch
            },
            prerelease,
        },
        major_wild,
        minor_wild,
        patch_wild,
    })
}

/// One version-core part: a wildcard (`x` / `X` / `*`) or a numeral
/// with no leading zero.
fn parse_part(text: &str) -> Option<(u64, bool)> {
    if matches!(text, "x" | "X" | "*") {
        return Some((0, true));
    }
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse::<u64>().ok().map(|n| (n, false))
}

fn is_pre_charset(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn parse_prerelease(pre: &str) -> Option<Vec<Identifier>> {
    pre.split('.')
        .map(|id| {
            if id.is_empty() {
                return None;
            }
            // Numeric only in strict semver form (no leading zeros);
            // anything else — including digit runs too big for u64 —
            // compares as an alphanumeric identifier, like TS's regex
            // classification does.
            if id.chars().all(|c| c.is_ascii_digit())
                && (id.len() == 1 || !id.starts_with('0'))
                && let Ok(n) = id.parse::<u64>()
            {
                return Some(Identifier::Numeric(n));
            }
            Some(Identifier::Alpha(id.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version::new(major, minor, patch)
    }

    fn v_pre(major: u64, minor: u64, patch: u64, pre: &str) -> Version {
        Version {
            prerelease: parse_prerelease(pre).unwrap(),
            ..Version::new(major, minor, patch)
        }
    }

    fn matches(range: &str, version: &Version) -> bool {
        VersionRange::try_parse(range).unwrap().test(version)
    }

    #[test]
    fn simple_bounds() {
        assert!(matches(">=5.2", &v(7, 0, 0)));
        assert!(!matches("<6", &v(7, 0, 0)));
        assert!(matches("<6", &v(5, 9, 9)));
        assert!(matches("=5.1.2", &v(5, 1, 2)));
        assert!(!matches("=5.1.2", &v(5, 1, 3)));
        assert!(matches("5.1.2", &v(5, 1, 2)));
    }

    #[test]
    fn wildcard_partials() {
        assert!(matches("5.x", &v(5, 3, 0)));
        assert!(!matches("5.x", &v(6, 0, 0)));
        assert!(matches("5.2.x", &v(5, 2, 9)));
        assert!(!matches("5.2.x", &v(5, 3, 0)));
        assert!(matches("*", &v(0, 0, 1)));
        assert!(matches("x", &v(99, 0, 0)));
    }

    #[test]
    fn tilde_and_caret() {
        assert!(matches("~5.2.0", &v(5, 2, 9)));
        assert!(!matches("~5.2.0", &v(5, 3, 0)));
        assert!(matches("^5.2.0", &v(5, 9, 0)));
        assert!(!matches("^5.2.0", &v(6, 0, 0)));
        assert!(matches("^0.2.0", &v(0, 2, 5)));
        assert!(!matches("^0.2.0", &v(0, 3, 0)));
    }

    #[test]
    fn hyphen_range() {
        assert!(matches("5.0 - 6.2", &v(5, 0, 0)));
        assert!(matches("5.0 - 6.2", &v(6, 2, 9)));
        assert!(!matches("5.0 - 6.2", &v(6, 3, 0)));
        assert!(matches("5.0.1 - 6.2.3", &v(6, 2, 3)));
        assert!(!matches("5.0.1 - 6.2.3", &v(6, 2, 4)));
    }

    #[test]
    fn disjunction_and_conjunction() {
        assert!(matches(">=5.0 <6 || >=7", &v(7, 5, 0)));
        assert!(matches(">=5.0 <6 || >=7", &v(5, 5, 0)));
        assert!(!matches(">=5.0 <6 || >=7", &v(6, 5, 0)));
    }

    #[test]
    fn empty_range_matches_everything() {
        assert!(matches("", &v(1, 0, 0)));
        assert!(matches("  ", &v(1, 0, 0)));
    }

    #[test]
    fn malformed_ranges_fail_to_parse() {
        assert!(VersionRange::try_parse("junk?").is_none());
        assert!(VersionRange::try_parse("5.2 || ?").is_none());
        // A whitespace-only piece between `||`s fails the grammar...
        assert!(VersionRange::try_parse("5.2 ||  || 6").is_none());
        assert!(VersionRange::try_parse("01.2.3").is_none());
        // ...but an empty TAIL piece is skipped (the input trims before
        // splitting), matching TS.
        assert!(VersionRange::try_parse(">=5.2 || ").is_some());
    }

    #[test]
    fn prerelease_inclusive_comparisons() {
        // Full-triple bound: a prerelease of the bound itself sits below it.
        assert!(!matches(">=7.0.0", &v_pre(7, 0, 0, "dev.20260416.1")));
        // Wildcard bound expands with `-0`, admitting prereleases.
        assert!(matches(">=7", &v_pre(7, 0, 0, "dev.20260416.1")));
        assert!(!matches("<7", &v_pre(7, 0, 0, "dev.20260416.1")));
        // Numeric identifiers rank below alphanumeric ones.
        assert!(v_pre(7, 0, 0, "0") < v_pre(7, 0, 0, "dev"));
        // Release outranks prerelease.
        assert!(v(7, 0, 0) > v_pre(7, 0, 0, "dev"));
    }

    #[test]
    fn unsatisfiable_wildcard_comparison() {
        assert!(!matches("<x", &v(1, 0, 0)));
        assert!(!matches(">*", &v(1, 0, 0)));
        assert!(matches("<=x", &v(1, 0, 0)));
    }
}
