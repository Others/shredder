use shredder::{Finalize, Gc, Scan};

#[derive(Scan, Finalize)]
struct Circular {
    self_ref: Gc<Circular>,
    n: u64,
}

#[test]
fn can_create_circular() {
    let circle: Gc<Circular> = Gc::new_circular(|gc| Circular {
        self_ref: gc,
        n: 146,
    });

    assert_eq!(circle.get().n, 146)
}
