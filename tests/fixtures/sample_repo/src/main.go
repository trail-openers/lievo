package main

import (
    "fmt"
    "os"
)

func processItems(items []int) []int {
    result := []int{}
    for _, item := range items {
        if item > 0 {
            result = append(result, item*2)
        }
    }
    return result
}

func loadConfig(path string) (string, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        return "", err
    }
    return string(data), nil
}
