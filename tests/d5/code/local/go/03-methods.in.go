package counter

type Counter struct {
	value int
}

func (c *Counter) Bump(step int) int {
	c.value += step
	return c.double(step)
}

func (c *Counter) double(step int) int {
	return step * 2
}
