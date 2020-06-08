extern crate shredder;

use std::fmt::Debug;

use shredder::GcSafe;
use shredder::R;
use shredder::Scan;

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
struct Test5 {
    i: u32,
    j: u32,
    #[shredder(skip)]
    k: NotScan,
}

#[derive(Scan)]
struct Test6 {
    i: u32,
    j: u32,
    #[shredder(unsafe_skip)]
    k: NotScan,
}

#[derive(Scan)]
struct Test7(#[shredder(unsafe_skip)] NotGcSafe);

#[derive(Scan)]
struct Test8<'a> {
    r: R<'a, str>
}

#[derive(Scan)]
struct Test9<'a, T> {
    r: R<'a, T>
}

#[derive(Scan)]
struct Test10<T: Scan + Debug> {
    v: T
}

fn main() {}
