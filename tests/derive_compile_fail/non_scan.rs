#[macro_use]
extern crate shredder;

struct NotScan {}

#[derive(Scan)]
struct Test0 {
    field: NotScan
}

fn main() {}
