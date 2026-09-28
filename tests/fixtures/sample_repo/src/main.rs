// The `rustmod` module chain (issue #724): `use crate::rustmod::deep::deep_label`
// below names a symbol in the nested module file src/rustmod/deep/mod.rs, a
// specifier the selfcheck independent resolver must not flag as wrong.
mod rustmod;

// The `steps` module (issue #732) declares its `cleanup` child with
// `#[path = "steps/cleanup.rs"]` inside src/steps.rs — the specifier below
// names a symbol in that #[path]-declared module, and the recorded edge must
// land on the physical file `src/steps/cleanup.rs`.
mod steps;

use crate::rustmod::deep::deep_label;
use crate::rustmod::module_name;
use crate::steps;
use crate::steps::cleanup::cleanup_label;
use std::collections::HashMap;

pub fn calculate_sum(items: &[i32]) -> i32 {
    items.iter().sum()
}

pub fn find_max(items: &[i32]) -> Option<i32> {
    if items.is_empty() {
        return None;
    }
    let mut max = items[0];
    for &item in items.iter() {
        if item > max {
            max = item;
        }
    }
    Some(max)
}

fn helper(x: i32) -> i32 {
    x * 2
}

pub fn labelled_sum(items: &[i32]) -> i32 {
    let label = deep_label();
    let module = module_name();
    let step = cleanup_label();
    println!("{} {} {}", label, module, step);
    items.iter().sum()
}
