// Child module of the nested fixture module `deep` (issue #724): defines
// the symbol that src/main.rs imports as
// `crate::rustmod::deep::deep_label` via the re-export in deep/mod.rs.
//
// `use super::super::module_name` (issue #742 task-a): a 2-hop super walk
// from this file. The 3-deep layout (src/rustmod.rs ← rustmod/deep/mod.rs
// ← this file) means the 1-hop form lands on src/rustmod/deep/mod.rs and
// the 2-hop form lands on src/rustmod.rs — DIFFERENT files, so the
// off-by-one defect (counting one leading super instead of two) cannot
// pass undetected.
use super::super::module_name;

pub fn deep_label() -> &'static str {
    "from deep module"
}

pub fn labelled() -> &'static str {
    module_name()
}
