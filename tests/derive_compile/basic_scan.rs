extern crate shredder;

use std::fmt::Debug;

use shredder::marker::{GcSafe, GcDrop};
use shredder::{Scan, R};

struct NotScan {}
unsafe impl GcSafe for NotScan {}

struct NotGcSafe {}

#[derive(Scan)]
struct Test0 {}

#[derive(Scan)]
struct Test1();

#[derive(Scan)]
struct Test2 {
    i: u32,
}

#[derive(Scan)]
struct Test3 {
    i: u32,
    j: u32,
    k: u32,
}

#[derive(Scan)]
struct Test4(u32, u32, u32);

#[derive(Scan)]
#[shredder(cant_drop)]
struct Test5 {
    i: u32,
    j: u32,
    #[shredder(skip_scan)]
    k: NotScan,
}

#[derive(Scan)]
struct Test6 {
    i: u32,
    j: u32,
    #[shredder(unsafe_skip_all)]
    k: NotScan,
}

#[derive(Scan)]
struct Test7(#[shredder(unsafe_skip_all)] NotGcSafe);

#[derive(Scan)]
#[shredder(cant_drop)]
struct Test8<'a> {
    r: R<'a, str>
}

#[derive(Scan)]
#[shredder(cant_drop)]
struct Test9<'a, T> {
    r: R<'a, T>
}

#[derive(Scan)]
struct Test10<T: Scan + GcDrop + Debug> {
    v: T
}

fn main() {}
