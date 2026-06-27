//! `animate:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Animation.ts`.
//!
//! Intentionally empty: `animate:` has no analyze-phase work. It
//! introduces no scope bindings and no template refs (the param
//! expression is emitted inline and resolved by TS). All handling
//! lives in `crates/emit/src/nodes/animation.rs`.
