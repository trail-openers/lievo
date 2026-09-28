// Go sample file for tree-sitter extraction integration test
package main

import (
	"fmt"
)

// Simple function with complexity 1
func hello() {
	fmt.Println("Hello from Go!")
}

// Function with branching (complexity > 1)
func processValue(value int) string {
	if value > 0 {
		return fmt.Sprintf("positive: %d", value)
	} else if value < 0 {
		return fmt.Sprintf("negative: %d", value)
	} else {
		return "zero"
	}
}

// Function with loop (increased complexity)
func sumNumbers(numbers []int) int {
	total := 0
	for _, num := range numbers {
		total += num
	}
	return total
}

// Function that calls other functions
func main() {
	hello()
	result := processValue(42)
	fmt.Println(result)
	nums := []int{1, 2, 3, 4, 5}
	total := sumNumbers(nums)
	fmt.Printf("Sum: %d\n", total)
}