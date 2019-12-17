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
