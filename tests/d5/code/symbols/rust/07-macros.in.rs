macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}

#[macro_export]
macro_rules! cube {
    ($x:expr) => {
        $x * $x * $x
    };
}

fn use_macros() -> i32 {
    square!(2) + cube!(2)
}
