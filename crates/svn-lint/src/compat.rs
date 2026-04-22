//! Version-gated rule behavior.
//!
//! Upstream svelte/compiler's warning rules evolve over minor
//! releases. Our native lint pass has a fixed ruleset baked in at
//! build time, which drifts from the user's installed `svelte/compiler`
//! as upstream ships new rules (or refines existing ones). Without
//! gating, we either over-fire (ruleset newer than user's svelte) or
//! under-fire (ruleset older).
//!
//! This module captures the known behavioral divergences as a
//! `CompatFeatures` struct, built once per run from the user's
//! detected `node_modules/svelte` version. Rules consult the flags
//! instead of hard-coding the latest behavior.
//!
//! ## Adding a new feature flag
//!
//! 1. Identify the upstream commit that changed the rule.
//! 2. Find the first tag containing it:
//!    `git tag --contains <sha>` in the svelte repo.
//! 3. Add a bool field named after the rule + behavior.
//! 4. Set it in [`CompatFeatures::from_version`] with the correct
//!    threshold.
//! 5. Consult `ctx.compat.YOUR_FLAG` at the rule site.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvelteVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SvelteVersion {
    /// Returns true when `self >= (major, minor, patch)` by SemVer
    /// precedence.
    pub fn at_least(self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }

    /// Parse a `MAJOR.MINOR.PATCH` string (optionally prefixed `v`,
    /// trailing `-prerelease` / `+build` stripped). Returns `None`
    /// when any segment isn't a plain integer.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let core = s.split(|c| c == '-' || c == '+').next().unwrap_or(s);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some(Self { major, minor, patch })
    }
}

/// Resolved per-rule feature flags. Defaults to `modern` (all known
/// gates on) when we can't detect the user's svelte version — matches
/// what the `upstream_validator` fixture suite enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompatFeatures {
    /// `a11y_no_static_element_interactions` fires on `onpointer*` /
    /// `ontouch*` event attributes. Upstream PR #17548 (commit
    /// `5872b89f`) first released in **svelte@5.48.3**. Before that,
    /// only keyboard / click / drag / mouse handlers counted.
    pub a11y_pointer_touch_handlers: bool,
    /// `state_referenced_locally` fires on reads of `rest_prop`
    /// bindings (e.g. `const props = $props(); props.x`). Upstream
    /// PR #17708 (commit `dd9fc0d1a`) first released in
    /// **svelte@5.51.2**. Before that, only `prop` kind fired.
    pub state_locally_rest_prop: bool,
}

impl CompatFeatures {
    /// All flags on — mirrors upstream main. Used when version
    /// detection fails, so behavior matches the validator suite.
    pub const MODERN: Self = Self {
        a11y_pointer_touch_handlers: true,
        state_locally_rest_prop: true,
    };

    pub fn from_version(v: Option<SvelteVersion>) -> Self {
        let Some(v) = v else { return Self::MODERN };
        Self {
            a11y_pointer_touch_handlers: v.at_least(5, 48, 3),
            state_locally_rest_prop: v.at_least(5, 51, 2),
        }
    }
}

impl Default for CompatFeatures {
    fn default() -> Self {
        Self::MODERN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_version() {
        let v = SvelteVersion::parse("5.48.2").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (5, 48, 2));
    }

    #[test]
    fn parses_prerelease_and_build_metadata() {
        let v = SvelteVersion::parse("5.49.0-next.3+sha.abc").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (5, 49, 0));
    }

    #[test]
    fn at_least_respects_semver_precedence() {
        let v = SvelteVersion { major: 5, minor: 48, patch: 3 };
        assert!(v.at_least(5, 48, 3));
        assert!(!v.at_least(5, 48, 4));
        assert!(v.at_least(5, 48, 2));
        assert!(!v.at_least(6, 0, 0));
    }

    #[test]
    fn bench_snapshot_thresholds() {
        // bench/control-svelte-4 pins svelte 5.48.2 — legacy ruleset.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5, minor: 48, patch: 2,
        }));
        assert!(!c.a11y_pointer_touch_handlers);
        assert!(!c.state_locally_rest_prop);

        // bench/control-svelte-5 pins svelte 5.55.4 — modern ruleset.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5, minor: 55, patch: 4,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_rest_prop);

        // Right at the threshold for pointer/touch.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5, minor: 48, patch: 3,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(!c.state_locally_rest_prop);

        // Right at the threshold for rest_prop.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5, minor: 51, patch: 2,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_rest_prop);
    }

    #[test]
    fn no_version_defaults_to_modern() {
        let c = CompatFeatures::from_version(None);
        assert_eq!(c, CompatFeatures::MODERN);
    }
}
