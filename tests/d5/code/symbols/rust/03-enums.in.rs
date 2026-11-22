pub enum Shape {
    Circle(f64),
    Square { side: f64 },
}

impl Shape {
    pub fn area(&self) -> f64 {
        match self {
            Shape::Circle(r) => 3.14 * r * r,
            Shape::Square { side } => side * side,
        }
    }
}
