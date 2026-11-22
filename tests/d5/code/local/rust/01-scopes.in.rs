use std::fmt::Display;

fn area(r: u32) -> u32 {
    3 * r * r
}

fn show<T: Display>(value: T) -> String {
    format!("{value}")
}

fn main() {
    let r = area(2);
    println!("{}", show(r));
}
