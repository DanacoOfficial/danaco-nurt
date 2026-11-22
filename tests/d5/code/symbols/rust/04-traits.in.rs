pub trait Draw {
    fn draw(&self);

    fn name(&self) -> String {
        String::from("shape")
    }
}

struct Canvas;

impl Draw for Canvas {
    fn draw(&self) {}
}
