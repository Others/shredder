use std::any::Any;

use shredder::marker::{GcDeref, GcDrop};
use shredder::{DerefGc, Gc, Scan, ToScan};

trait Super: Scan + ToScan + GcDrop + GcDeref + Any {}

#[derive(Scan)]
#[shredder(can_deref)]
struct Sub;

impl Super for Sub {}

#[derive(Scan)]
#[shredder(can_deref)]
struct NotSub;

#[test]
fn can_downcast_sub_test() {
    let sub = Box::new(Sub);
    let sub = sub as Box<dyn Super>;

    let gc = Gc::from_box(sub);
    let v = gc.downcast::<Sub>();
    assert!(v.is_some());
}

#[test]
fn cant_downcast_not_sub_test() {
    let sub = Box::new(Sub);
    let sub = sub as Box<dyn Super>;

    let gc = Gc::from_box(sub);
    let v = gc.downcast::<NotSub>();
    assert!(v.is_none());
}

#[test]
fn can_downcast_deref_gc_sub_test() {
    let sub = Box::new(Sub);
    let sub = sub as Box<dyn Super>;

    let gc = DerefGc::from_box(sub);
    let v = gc.downcast::<Sub>();
    assert!(v.is_some());
}

#[test]
fn cant_downcast_deref_gc_not_sub_test() {
    let sub = Box::new(Sub);
    let sub = sub as Box<dyn Super>;

    let gc = DerefGc::from_box(sub);
    let v = gc.downcast::<NotSub>();
    assert!(v.is_none());
}
