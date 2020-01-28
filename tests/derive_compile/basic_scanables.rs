extern crate shredder;

use shredder::Scan;
use shredder::GcSafe;

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

fn main() {}
