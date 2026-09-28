// Rust sample file providing a known cross-file call chain for
// graph-centrality (Personalized PageRank) integration tests.
//
// The intended edge chain (extracted from the call sites below) is:
//
//     entry_point -> middle_step -> leaf_step
//
// so a centrality producer seeded on `entry_point` can be asserted to
// rank the chain members in call-distance order against a real
// tree-sitter-extracted relationship graph.

use crate::rust_helper::helper_label;

fn leaf_step(input: i32) -> i32 {
    if input > 0 {
        input + 1
    } else {
        input - 1
    }
}

fn middle_step(input: i32) -> i32 {
    let doubled = input * 2;
    leaf_step(doubled)
}

fn entry_point(input: i32) -> i32 {
    let labelled = helper_label();
    println!("{labelled}");
    middle_step(input)
}
