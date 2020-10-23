extern crate shredder;

use shredder::Scan;

struct NotScan {}

#[derive(Scan)]
struct Test0 {
    field: NotScan
}

fn main() {}
