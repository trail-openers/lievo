// Parent module of the nested fixture module `deep` (issue #724). Declared
// as `mod rustmod;` from src/main.rs so the `crate::rustmod::deep` module
// chain resolves.
//
// Defines and forwards its own symbol (`module_name`) so this file has a
// recorded import edge — a `pub mod deep;`-only re-export file would have
// none, which the selfcheck false_zero_callers section (b) would flag as a
// false-0-caller (its stem `rustmod` appears in main.rs) and fail the gate
// at the default threshold of 0.
//
// `module_name` is the target of the relative imports in this chain
// (issue #742 task-a): deep/mod.rs imports `super::module_name` (1 hop)
// and deep/label.rs imports `super::super::module_name` (2 hops) — both
// resolve to this file.
pub mod deep;

pub fn module_name() -> &'static str {
    "rustmod"
}
