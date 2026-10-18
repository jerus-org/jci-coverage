fn plain() {}

fn pick(x: Option<i32>) -> Option<i32> {
    let v = x?;
    match v {
        0 => None,
        _ => Some(v),
    }
}
