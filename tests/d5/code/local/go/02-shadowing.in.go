package main

var value = 1

func shadow(value int) int {
	return value + 1
}

func loop(items []int) int {
	total := 0
	for _, item := range items {
		total += item
	}
	return total
}
