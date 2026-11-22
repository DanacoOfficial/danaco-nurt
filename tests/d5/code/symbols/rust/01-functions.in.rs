fn main() {
    println!("{}", add(1, 2));
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub(crate) fn internal() {}
