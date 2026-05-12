//! Filename-parity mirror for upstream
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/`.
//!
//! Most files in this directory carry real logic — extracted out of
//! the `process_instance_script_content.rs` mega-file so they match
//! upstream's per-concern split. A few remain as pointer-only stubs
//! where the concern is genuinely centralized elsewhere in our tree
//! (cross-crate `analyze` ownership, etc.).
//!
//! ## Live modules (real code)
//!
//! | Upstream | Our file | Notes |
//! |---|---|---|
//! | (half of) `ExportedNames.ts` | `exported_type_info.rs` | `collect_export_type_infos` — declaration → `ExportedLocalInfo` |
//! | `HoistableInterfaces.ts` (typeof scan part) | `type_refs.rs` | `keyof_typeof_targets`, `typeof_targets` |
//! | `InterfacesAndTypes.ts` | `ident_refs.rs` | `collect_ident_refs` — reference scanner for hoistability |
//!
//! ## Pointer-only stubs (logic lives elsewhere)
//!
//! | Upstream | Our file | Actual impl |
//! |---|---|---|
//! | `ComponentDocumentation.ts` | `component_documentation.rs` | not implemented (CLI has no LSP consumer) |
//! | `ComponentEvents.ts` | `component_events.rs` | `analyze::walker` + `nodes::inline_component::emit_on_event_calls` |
//! | `ExportedNames.ts` (collection + emit half) | `exported_names.rs` | `process_instance_script_content::ExportedLocalInfo` + `props_emit::build_exports_object` |
//! | `Generics.ts` | `generics.rs` | `util::generic_arg_names` + `default_export.rs` class wrapper |
//! | `ImplicitStoreValues.ts` | `implicit_store_values.rs` | `analyze::store` |
//!
//! ## Upstream files we don't surface here
//!
//! | Upstream | Why no file |
//! |---|---|
//! | `event-handler.ts` | overlaps with `htmlxtojsx_v2/nodes/EventHandler.ts`; we have `nodes::event_handler` already |
//! | `handleImportDeclaration.ts` | inline in `process_instance_script_content::split_imports` |
//! | `handleScopeAndResolveForSlot.ts` | inline in `nodes::let_directive` |
//! | `handleTypeAssertion.ts` | inline in `process_instance_script_content` |
//! | `HoistableInterfaces.ts` (decision logic) | inline in `process_instance_script_content::hoisted_type_names` (only the typeof scan was extracted) |
//! | `ImplicitTopLevelNames.ts` | inline in `analyze::collect_top_level_bindings` |
//! | `Scripts.ts` | inline in `process_instance_script_content` and `process_module_script_tag` |
//! | `slot.ts` | overlaps with `nodes::let_directive::slot_let_attrs` etc. |
//! | `Stores.ts` | overlaps with `ImplicitStoreValues.ts`; both pointers go to `analyze::store` |
//! | `TemplateScope.ts` | `analyze::template_scope` |

pub(crate) mod component_documentation;
pub(crate) mod component_events;
pub(crate) mod exported_names;
pub(crate) mod exported_type_info;
pub(crate) mod generics;
pub(crate) mod ident_refs;
pub(crate) mod implicit_store_values;
pub(crate) mod type_refs;
