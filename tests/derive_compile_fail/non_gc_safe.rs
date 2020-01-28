#[macro_use]
extern crate shredder;

struct NotGcSafe {}

#[derive(Scan)]
struct Test0 {
    #[shredder(skip)]
    field: NotGcSafe
}

fn main() {}
