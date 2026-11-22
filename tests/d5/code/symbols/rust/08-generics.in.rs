pub type Result<T> = std::result::Result<T, String>;

pub struct Wrapper<T>(T);

impl<T: Clone> Wrapper<T> {
    pub fn get(&self) -> T {
        self.0.clone()
    }
}

pub fn first<T>(items: &[T]) -> Option<&T> {
    items.first()
}
