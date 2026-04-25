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
//! [`CompatFeatures`] struct, built once per run from the user's
//! detected `node_modules/svelte` version. Rules consult the flags
//! instead of hard-coding the latest behavior. [`detect_for_workspace`]
//! is the single detection entry point — CLI and integration tests
//! both route through it so the fallback semantics are consistent.
//!
//! ## Adding a new feature flag
//!
//! 1. Identify the upstream commit that changed the rule.
//! 2. Find the first tag containing it:
//!    `git tag --contains <sha>` in the svelte repo.
//! 3. Add a bool field named after the rule + behavior.
//! 4. Set it in [`CompatFeatures::from_version`] with the correct
//!    threshold. Include the upstream PR URL in the doc comment.
//! 5. Consult `ctx.compat.YOUR_FLAG` at the rule site.

use std::path::Path;

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
        let core = s.split(['-', '+']).next().unwrap_or(s);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

/// Resolved per-rule feature flags. Defaults to `modern` (all known
/// gates on) when we can't detect the user's svelte version — matches
/// what the `upstream_validator` fixture suite enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompatFeatures {
    /// `a11y_no_static_element_interactions` fires on `onpointer*` /
    /// `ontouch*` event attributes. Before this, only keyboard /
    /// click / drag / mouse handlers counted.
    ///
    /// - Upstream PR: <https://github.com/sveltejs/svelte/pull/17548>
    /// - Commit: `5872b89f`
    /// - First released in **svelte@5.48.3**.
    pub a11y_pointer_touch_handlers: bool,
    /// `state_referenced_locally` fires on `prop` / `bindable_prop`
    /// bindings at all. Before this, only `state` / `raw_state` /
    /// `derived` fired — reading a regular destructured prop at
    /// top-level didn't warn.
    ///
    /// - Upstream PR: <https://github.com/sveltejs/svelte/pull/17266>
    /// - Commit: `570f64963`
    /// - First released in **svelte@5.45.3**.
    pub state_locally_fires_on_props: bool,
    /// `state_referenced_locally` fires on reads of `rest_prop`
    /// bindings (e.g. `const props = $props(); props.x`). Before
    /// this, only `prop` kind fired. Has no effect when
    /// `state_locally_fires_on_props` is false.
    ///
    /// - Upstream PR: <https://github.com/sveltejs/svelte/pull/17708>
    /// - Commit: `dd9fc0d1a`
    /// - First released in **svelte@5.51.2**.
    pub state_locally_rest_prop: bool,
}

impl CompatFeatures {
    /// All flags on — mirrors upstream main. Used when version
    /// detection fails, so behavior matches the validator suite.
    pub const MODERN: Self = Self {
        a11y_pointer_touch_handlers: true,
        state_locally_fires_on_props: true,
        state_locally_rest_prop: true,
    };

    pub fn from_version(v: Option<SvelteVersion>) -> Self {
        let Some(v) = v else { return Self::MODERN };
        Self {
            a11y_pointer_touch_handlers: v.at_least(5, 48, 3),
            state_locally_fires_on_props: v.at_least(5, 45, 3),
            state_locally_rest_prop: v.at_least(5, 51, 2),
        }
    }
}

impl Default for CompatFeatures {
    fn default() -> Self {
        Self::MODERN
    }
}

/// Resolve [`CompatFeatures`] for a workspace by locating the user's
/// installed `svelte` package and reading its `version` field.
///
/// Walks upward from `workspace` looking for
/// `node_modules/svelte/package.json`. Handles both flat npm/yarn
/// layouts and pnpm's `.pnpm/svelte@<ver>/node_modules/svelte/`
/// content-addressed layout (a flat-style symlink at the root still
/// exists in that case).
///
/// Falls back to [`CompatFeatures::MODERN`] when:
/// - `node_modules/svelte/package.json` isn't found at all, or
/// - `version` is missing / unparseable.
///
/// The fallback matches what our `upstream_validator` fixture suite
/// enforces, so zero-config workspaces and tests stay in sync.
pub fn detect_for_workspace(workspace: &Path) -> CompatFeatures {
    CompatFeatures::from_version(locate_svelte_version(workspace))
}

/// Walk up from `start` looking for `node_modules/svelte/package.json`;
/// return its parsed semver. Exposed for callers that want the raw
/// version for logging / diagnostics; most callers should use
/// [`detect_for_workspace`] which folds detection + gating into one
/// call.
pub fn locate_svelte_version(start: &Path) -> Option<SvelteVersion> {
    svn_core::walk_up_dirs(start, |dir| {
        let pkg = dir
            .join(svn_core::NODE_MODULES_DIR)
            .join("svelte")
            .join("package.json");
        pkg.is_file().then(|| read_package_version(&pkg)).flatten()
    })
}

/// Extract the `version` field from a `package.json`. Stringly
/// parsed — avoids pulling serde_json just for one key.
fn read_package_version(path: &Path) -> Option<SvelteVersion> {
    let text = std::fs::read_to_string(path).ok()?;
    let idx = text.find("\"version\"")?;
    let rest = &text[idx + 9..];
    let colon = rest.find(':')?;
    let after = &rest[colon + 1..];
    let start = after.find('"')?;
    let tail = &after[start + 1..];
    let end = tail.find('"')?;
    SvelteVersion::parse(&tail[..end])
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
        let v = SvelteVersion {
            major: 5,
            minor: 48,
            patch: 3,
        };
        assert!(v.at_least(5, 48, 3));
        assert!(!v.at_least(5, 48, 4));
        assert!(v.at_least(5, 48, 2));
        assert!(!v.at_least(6, 0, 0));
    }

    #[test]
    fn bench_snapshot_thresholds() {
        // A real bench pins svelte 5.20.5 — pre-props ruleset.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 20,
            patch: 5,
        }));
        assert!(!c.a11y_pointer_touch_handlers);
        assert!(!c.state_locally_fires_on_props);
        assert!(!c.state_locally_rest_prop);

        // Our Svelte-4 control bench pins svelte 5.48.2 —
        // post-props, pre-pointer-touch, pre-rest-prop.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 48,
            patch: 2,
        }));
        assert!(!c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_fires_on_props);
        assert!(!c.state_locally_rest_prop);

        // Our Svelte-5 control bench pins svelte 5.55.4 — modern ruleset.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 55,
            patch: 4,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_fires_on_props);
        assert!(c.state_locally_rest_prop);

        // Right at the threshold for props in state-locally.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 45,
            patch: 3,
        }));
        assert!(c.state_locally_fires_on_props);
        assert!(!c.a11y_pointer_touch_handlers);
        assert!(!c.state_locally_rest_prop);

        // Right at the threshold for pointer/touch.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 48,
            patch: 3,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_fires_on_props);
        assert!(!c.state_locally_rest_prop);

        // Right at the threshold for rest_prop.
        let c = CompatFeatures::from_version(Some(SvelteVersion {
            major: 5,
            minor: 51,
            patch: 2,
        }));
        assert!(c.a11y_pointer_touch_handlers);
        assert!(c.state_locally_fires_on_props);
        assert!(c.state_locally_rest_prop);
    }

    #[test]
    fn no_version_defaults_to_modern() {
        let c = CompatFeatures::from_version(None);
        assert_eq!(c, CompatFeatures::MODERN);
    }
}
