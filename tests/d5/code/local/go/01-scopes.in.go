package main

import "fmt"

func area(r int) int {
	return 3 * r * r
}

func main() {
	r := area(2)
	fmt.Println(r)
}
