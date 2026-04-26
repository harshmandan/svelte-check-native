//! Filename-parity stubs for upstream
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/`.
//!
//! Each file in this directory is a documentation pointer to where
//! the equivalent logic lives in our tree. Our architecture
//! centralizes most of these concerns into the `analyze` crate (see
//! `crates/analyze/src/`) and a few inline helpers in `emit`; the
//! stubs here exist purely so a contributor familiar with upstream's
//! filename layout can grep `svelte2tsx/nodes/<X>.ts` and find a
//! same-named file in our tree with a pointer to the actual impl.
//!
//! ## Mapping
//!
//! Implemented as stubs here:
//!
//! | Upstream | Our pointer file | Actual impl |
//! |---|---|---|
//! | `ComponentDocumentation.ts` | `component_documentation.rs` | not implemented (see file) |
//! | `ComponentEvents.ts` | `component_events.rs` | `analyze::template_walker` + `inline_component::emit_on_event_calls` |
//! | `ExportedNames.ts` | `exported_names.rs` | `process_instance_script_content::ExportedLocalInfo` |
//! | `Generics.ts` | `generics.rs` | `util::generic_arg_names` + `default_export.rs` class wrapper |
//! | `ImplicitStoreValues.ts` | `implicit_store_values.rs` | `analyze::store` |
//!
//! Other upstream files in `svelte2tsx/nodes/` not stubbed here:
//!
//! | Upstream | Why no stub |
//! |---|---|
//! | `event-handler.ts` | overlaps with `htmlxtojsx_v2/nodes/EventHandler.ts`; we have `nodes::event_handler` already |
//! | `handleImportDeclaration.ts` | inline in `process_instance_script_content::split_imports` |
//! | `handleScopeAndResolveForSlot.ts` | inline in `nodes::let_directive` |
//! | `handleTypeAssertion.ts` | inline in `process_instance_script_content` |
//! | `HoistableInterfaces.ts` | inline in `process_instance_script_content::hoisted_type_names` |
//! | `ImplicitTopLevelNames.ts` | inline in `analyze::collect_top_level_bindings` |
//! | `InterfacesAndTypes.ts` | inline in `process_instance_script_content` |
//! | `Scripts.ts` | inline in `process_instance_script_content` and `process_module_script_tag` |
//! | `slot.ts` | overlaps with `nodes::let_directive::slot_let_attrs` etc. |
//! | `Stores.ts` | overlaps with `ImplicitStoreValues.ts`; both pointers go to `analyze::store` |
//! | `TemplateScope.ts` | `analyze::template_scope` |

pub(crate) mod component_documentation;
pub(crate) mod component_events;
pub(crate) mod exported_names;
pub(crate) mod generics;
pub(crate) mod implicit_store_values;
