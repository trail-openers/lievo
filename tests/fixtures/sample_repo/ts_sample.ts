// TypeScript sample file for tree-sitter extraction integration test

interface NumberArray {
  numbers: number[];
}

// Simple function with complexity 1
function hello(): void {
  console.log("Hello from TypeScript!");
}

// Function with branching (complexity > 1)
function processValue(value: number): string {
  if (value > 0) {
    return `positive: ${value}`;
  } else if (value < 0) {
    return `negative: ${value}`;
  } else {
    return "zero";
  }
}

// Function with loop (increased complexity)
function sumNumbers(numbers: number[]): number {
  let total = 0;
  for (const num of numbers) {
    total += num;
  }
  return total;
}

// Function that calls other functions
function main(): void {
  hello();
  const result = processValue(42);
  console.log(result);
  const nums: number[] = [1, 2, 3, 4, 5];
  const total = sumNumbers(nums);
  console.log(`Sum: ${total}`);
  console.log(triple(7));
}

// Arrow function bound to a const — should be named "triple" (not the parameter)
const triple = (x: number): number => x * 3;

main();