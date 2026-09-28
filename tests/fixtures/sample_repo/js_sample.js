// JavaScript sample file for tree-sitter extraction integration test

// Relative import that should resolve to ts_sample.ts via the "source" field
// (the bare path, without quotes or the rest of the statement).
import { sumNumbers } from './ts_sample';

// Simple function with complexity 1
function hello() {
    console.log("Hello from JavaScript!");
}

// Function with branching (complexity > 1)
function processValue(value) {
    if (value > 0) {
        return `positive: ${value}`;
    } else if (value < 0) {
        return `negative: ${value}`;
    } else {
        return "zero";
    }
}

// Function with loop (increased complexity)
function sumNumbers(numbers) {
    let total = 0;
    for (const num of numbers) {
        total += num;
    }
    return total;
}

// Arrow function bound to a const — should be named "double" (not "x")
const double = (x) => x * 2;

// Arrow function with a destructured parameter — the arrow node has no bare
// identifier child at all, so the name must come from the binding "Pagination"
const Pagination = ({ page }) => {
    return page > 0 ? page : 0;
};

// Arrow function with no parameters — should be named "noArg"
const noArg = () => 42;

// Arrow function with a named parameter — must be named "increment", not "prev"
const increment = (prev) => prev + 1;

// Class with a method — the method should be named "greet", not a parameter
class Greeting {
    greet(name) {
        return `Hello ${name}`;
    }
}

// Function that calls other functions
function main() {
    hello();
    const result = processValue(42);
    console.log(result);
    const nums = [1, 2, 3, 4, 5];
    const total = sumNumbers(nums);
    console.log(`Sum: ${total}`);
    console.log(double(21));
    console.log(Pagination({ page: 3 }));
    console.log(noArg());
    console.log(increment(1));
    const g = new Greeting();
    console.log(g.greet("world"));
}

main();