package main

var handler = func() {}

func counter() func() int {
	n := 0
	return func() int {
		n++
		return n
	}
}
