// 3-deep fixture chain (issue #742 task-a): this file is the directory-module
// form (src/rustmod/deep/mod.rs) of the module `crate::rustmod::deep`.
//
// `deep_label` is defined in src/rustmod/deep/label.rs (a child module of
// this one), which the parent module file re-exports below so that
// `crate::rustmod::deep::deep_label` names the symbol even though the
// literal file src/rustmod/deep/deep_label.rs does not exist — the exact
// symbol-vs-file mismatch issue #724 fixes.
//
// `use super::module_name` (issue #742) imports the PARENT module
// (`crate::rustmod`, file src/rustmod.rs) — a 1-hop walk from this
// mod.rs-form file. The 2-hop form lives in label.rs (super::super::) so
// the two specifiers land on DIFFERENT files on this layout.
mod label;

pub use label::deep_label;

use super::module_name;

pub fn deep_tag() -> String {
    format!("deep:{}", module_name())
}
