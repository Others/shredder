extern crate shredder;

use shredder::Scan;
use shredder::marker::GcDrop;

struct NotGcSafe {}
unsafe impl GcDrop for NotGcSafe {}

#[derive(Scan)]
struct Test0 {
    #[shredder(skip_scan)]
    field: NotGcSafe
}

fn main() {}
