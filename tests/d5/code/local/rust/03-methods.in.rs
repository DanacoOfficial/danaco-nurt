struct Counter {
    value: u32,
}

impl Counter {
    fn bump(&mut self, step: u32) -> u32 {
        self.value += step;
        self.double(step)
    }

    fn double(&self, step: u32) -> u32 {
        step * 2
    }
}
