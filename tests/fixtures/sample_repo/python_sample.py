# Python sample file for tree-sitter extraction integration test
from typing import List
import os

# Simple function with complexity 1
def hello():
    print("Hello from Python!")

# Function with branching (complexity > 1)
def process_value(value: int) -> str:
    if value > 0:
        return f"positive: {value}"
    elif value < 0:
        return f"negative: {value}"
    else:
        return "zero"

# Function with loop (increased complexity)
def sum_numbers(numbers: List[int]) -> int:
    total = 0
    for num in numbers:
        total += num
    return total

# Function that calls other functions
def main():
    hello()
    result = process_value(42)
    print(result)
    nums = [1, 2, 3, 4, 5]
    total = sum_numbers(nums)
    print(f"Sum: {total}")

if __name__ == "__main__":
    main()