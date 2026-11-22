package shapes

type Shape interface {
	Area() float64
	Perimeter() float64
}

type reader interface {
	Read(p []byte) (int, error)
}
