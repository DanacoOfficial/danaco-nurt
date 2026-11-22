pub async fn fetch() -> u32 {
    1
}

pub unsafe fn raw() {}

extern "C" fn callback() {}

union Bits {
    int: u32,
    float: f32,
}
