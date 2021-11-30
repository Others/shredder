use rayon::iter::{IntoParallelRefMutIterator, ParallelBridge, ParallelIterator};
use std::cell::RefCell;
use thread_local::ThreadLocal;

pub(crate) struct CrossThreadBuffer<T: Send> {
    buffers: ThreadLocal<RefCell<Vec<T>>>,
}

impl<T: Send> CrossThreadBuffer<T> {
    pub fn new() -> Self {
        Self {
            buffers: ThreadLocal::with_capacity(num_cpus::get()),
        }
    }

    pub fn push(&self, item: T) {
        let tlb = self.buffers.get_or_default();
        tlb.borrow_mut().push(item);
    }

    pub fn clear(&mut self) {
        for v in self.buffers.iter_mut() {
            v.get_mut().clear();
        }
    }

    pub fn par_for_each<F: Fn(&mut T) + Send + Sync>(&mut self, f: F) {
        self.buffers.iter_mut().par_bridge().for_each(|vec| {
            vec.borrow_mut().par_iter_mut().for_each(|mut x| f(&mut x));
        })
    }
}

impl<T: Send> Default for CrossThreadBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}
