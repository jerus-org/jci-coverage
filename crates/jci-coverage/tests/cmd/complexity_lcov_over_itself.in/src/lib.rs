use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

pub fn pick<T: Default + PartialEq>(x: T) -> bool {
    if x == T::default() { false } else { true }
}

pub fn total(v: &[i32]) -> i32 {
    v.iter().map(|x| if *x > 1 { *x } else { 0 }).sum()
}

pub async fn later(n: u32) -> u32 {
    if n > 1 { n * 2 } else { n }
}

pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

pub fn never() -> u8 {
    7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs() {
        assert!(pick(1u8));
        assert!(!pick(0u16));
        assert_eq!(total(&[1, 2, 3]), 5);
        assert_eq!(block_on(later(3)), 6);
    }
}
