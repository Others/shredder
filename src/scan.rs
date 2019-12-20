use crate::collector::GcInternalHandle;
use crate::Gc;

pub unsafe trait Scan {
    // TODO: Consider if a HashSet would be a better fit
    fn scan(&self, out: &mut Vec<GcInternalHandle>);
}

unsafe impl Scan for i32 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for u32 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for i64 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for u64 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl<T: Scan> Scan for Gc<T> {
    fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        out.push(self.internal_handle())
    }
}

unsafe impl<T: Scan> Scan for Vec<T> {
    fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        for v in self {
            v.scan(out);
        }
    }
}

// TODO: Add more Scan impls
// TODO: Add a Scan auto-derive

// TODO: Consider what happens if there are reference cycles (like a Gc -> Rc<A> -> A -> Rc<B> -> B -> Rc<A>)
// This could lead to an infinite loop during scanning
// To fix this, we'd have to change how the scan type works, with broadly three options
// - Keep track of visited items during scanning internally
// - Return a vector of Scan children instead of GcInternalHandle
// - Make Rc/Arc not Scan-able
