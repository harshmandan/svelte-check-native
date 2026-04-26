//! SvelteKit-file discovery — re-export shim.
//!
//! The classification logic now lives in `svn_core::sveltekit`. This
//! file remains as a thin re-export so existing
//! `use crate::kit_files::KitFilesSettings;` and `mod kit_files;`
//! sites in `cli/main.rs` and `cli/svelte_config.rs` keep compiling
//! while phases 3-6 of `notes/PLAN-sveltekit-path-centralization.md`
//! migrate the rest of the workspace onto the centralized primitive.
//!
//! Once every consumer imports `svn_core::sveltekit` directly, this
//! file folds away.

pub use svn_core::sveltekit::KitFilesSettings;
