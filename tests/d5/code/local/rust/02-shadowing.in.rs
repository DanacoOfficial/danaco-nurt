fn shadow(value: u32) -> u32 {
    let value = value + 1;
    let value = value * 2;
    value
}

fn closure() -> impl Fn(u32) -> u32 {
    let offset = 3;
    move |x| x + offset
}
