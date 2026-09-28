// Rust sample file for tree-sitter extraction integration test
mod rust_helper;

use crate::rust_helper::helper_label;
use std::collections::HashMap;

// Simple function with complexity 1
fn hello() {
    println!("Hello from Rust!");
}

// Function with branching (complexity > 1)
fn process_value(value: i32) -> String {
    if value > 0 {
        format!("positive: {}", value)
    } else if value < 0 {
        format!("negative: {}", value)
    } else {
        "zero".to_string()
    }
}

// Function with loop (increased complexity)
fn sum_numbers(numbers: &[i32]) -> i32 {
    let mut sum = 0;
    for num in numbers {
        sum += num;
    }
    sum
}

// Function that imports and calls other functions
fn main() {
    hello();
    let result = process_value(42);
    println!("{}", result);
    let helper = helper_label();
    println!("{}", helper);
    let nums = vec![1, 2, 3, 4, 5];
    let total = sum_numbers(&nums);
    println!("Sum: {}", total);
}