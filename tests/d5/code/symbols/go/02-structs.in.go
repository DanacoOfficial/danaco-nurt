package geometry

type Point struct {
	X, Y int
}

func (p Point) Norm() int {
	return p.X*p.X + p.Y*p.Y
}

func (p *Point) Scale(factor int) {
	p.X *= factor
	p.Y *= factor
}
