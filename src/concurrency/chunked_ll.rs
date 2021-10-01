use std::mem::{self, MaybeUninit};
use std::prelude::v1::*;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;

use arc_swap::{ArcSwapOption, Guard};
use crossbeam::queue::SegQueue;

const CHUNK_SIZE: usize = 1024;

/// It's a linked list of chunks, with an associated free list!
/// Note that there is a major limitation: the backing memory is never deallocated
/// (That means this data structure is only useful for globals)
#[derive(Debug)]
pub struct ChunkedLinkedList<T> {
    /// basically a free-queue storing pointers to chunks + indexes where there is an empty spot
    free_entries: SegQueue<(*const Chunk<T>, usize)>,
    /// head of linked list of data storing chunks
    head: AtomicPtr<Chunk<T>>,
    /// an estimate of how many items are in this linked list
    estimated_len: AtomicUsize,
}

unsafe impl<T> Send for ChunkedLinkedList<T> where T: Send + Sync {}
unsafe impl<T> Sync for ChunkedLinkedList<T> where T: Send + Sync {}

struct Chunk<T> {
    values: [ArcSwapOption<T>; CHUNK_SIZE],
    next: *const Chunk<T>,
}

unsafe impl<T> Send for Chunk<T> where T: Send {}
unsafe impl<T> Sync for Chunk<T> where T: Sync {}

impl<T> Chunk<T> {
    fn iter_this<F: Fn(Arc<T>) + Sync>(&self, f: &F) {
        for i in 0..CHUNK_SIZE {
            let v = Guard::into_inner(self.values[i].load());
            if let Some(arc) = v {
                f(arc)
            }
        }
    }

    fn par_iter_rest<F: Fn(Arc<T>) + Sync>(&self, f: &F)
    where
        T: Send + Sync,
    {
        if self.next.is_null() {
            self.iter_this(f);
        } else {
            rayon::join(
                || self.iter_this(f),
                || {
                    let next = unsafe { &*self.next };

                    next.par_iter_rest(f)
                },
            );
        }
    }

    fn retain_this<F: Fn(&Arc<T>) -> bool + Sync>(&self, f: &F, host: &ChunkedLinkedList<T>)
    where
        T: Send + Sync,
    {
        for i in 0..CHUNK_SIZE {
            let current = self.values[i].load();
            let should_retain = match &*current {
                Some(arc) => f(arc),
                None => true,
            };

            if !should_retain {
                let res = self.values[i].compare_and_swap(&current, None);
                if let (Some(res_ref), Some(cur_ref)) = (res.as_ref(), current.as_ref()) {
                    if Arc::as_ptr(res_ref) == Arc::as_ptr(cur_ref) {
                        host.estimated_len.fetch_sub(1, Ordering::Relaxed);
                        host.free_entries.push((self as _, i))
                    }
                }
            }
        }
    }

    fn par_retain_rest<F: Fn(&Arc<T>) -> bool + Sync>(&self, f: &F, host: &ChunkedLinkedList<T>)
    where
        T: Send + Sync,
    {
        if self.next.is_null() {
            self.retain_this(f, host);
        } else {
            let next = unsafe { &*self.next };

            rayon::join(
                || self.retain_this(f, host),
                || next.par_retain_rest(f, host),
            );
        }
    }
}

#[derive(Debug)]
pub struct CLLItem<T> {
    pub v: Arc<T>,
    from: *const Chunk<T>,
    idx: usize,
}

unsafe impl<T> Send for CLLItem<T> {}
unsafe impl<T> Sync for CLLItem<T> {}

impl<T> Clone for CLLItem<T> {
    fn clone(&self) -> Self {
        Self {
            v: self.v.clone(),
            from: self.from,
            idx: self.idx,
        }
    }
}

impl<T> ChunkedLinkedList<T> {
    pub fn new() -> Self {
        let free_entries = SegQueue::new();

        let head = Box::into_raw(Box::new(Chunk {
            values: initialize_values(),
            next: ptr::null(),
        }));

        for i in 0..CHUNK_SIZE {
            free_entries.push((head as *const _, i));
        }

        Self {
            free_entries,
            head: AtomicPtr::new(head as *mut _),
            estimated_len: AtomicUsize::new(0),
        }
    }

    fn expand(&self) {
        let mut new_head;
        loop {
            let old_head = self.head.load(Ordering::Relaxed);

            new_head = Box::into_raw(Box::new(Chunk {
                values: initialize_values(),
                next: old_head,
            }));

            let swap_result = self.head.compare_exchange(
                old_head,
                new_head,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );

            if swap_result.is_ok() {
                break;
            }

            // Get rid of that memory we allocated (in this case we failed to use it)
            // TODO: Just write over it instead of deallocating it
            unsafe {
                Box::from_raw(new_head);
            }
        }

        for i in 0..CHUNK_SIZE {
            self.free_entries.push((new_head as *const _, i));
        }
    }

    #[allow(clippy::redundant_else)]
    pub fn insert(&self, v: Arc<T>) -> CLLItem<T> {
        loop {
            if let Some(idx) = self.free_entries.pop() {
                let chunk = unsafe { &*idx.0 };
                let slot = &chunk.values[idx.1];

                // We know the slot is free because it's in the free list
                slot.store(Some(v.clone()));

                let res = CLLItem {
                    v,
                    from: idx.0,
                    idx: idx.1,
                };

                self.estimated_len.fetch_add(1, Ordering::Relaxed);

                return res;
            } else {
                self.expand();
            }
        }
    }

    pub fn remove(&self, cll_item: &CLLItem<T>) {
        let chunk = unsafe { &*cll_item.from };

        let slot = &chunk.values[cll_item.idx];
        let res = slot.compare_and_swap(&cll_item.v, None);

        if let Some(prev_v) = &*res {
            if Arc::as_ptr(prev_v) == Arc::as_ptr(&cll_item.v) {
                // We did a remove, so record that swap
                self.estimated_len.fetch_sub(1, Ordering::Relaxed);

                self.free_entries.push((cll_item.from, cll_item.idx))
            }
        }
    }

    pub fn par_retain<F: Fn(&Arc<T>) -> bool + Sync>(&self, f: F)
    where
        T: Send + Sync,
    {
        let head = unsafe { &*self.head.load(Ordering::Relaxed) };
        head.par_retain_rest(&f, self);
    }

    pub fn par_iter<F: Fn(Arc<T>) + Sync>(&self, f: F)
    where
        T: Send + Sync,
    {
        let head = unsafe { &*self.head.load(Ordering::Relaxed) };
        head.par_iter_rest(&f);
    }

    pub fn estimate_len(&self) -> usize {
        self.estimated_len.load(Ordering::Relaxed)
    }
}

fn initialize_values<T>() -> [ArcSwapOption<T>; CHUNK_SIZE] {
    unsafe {
        let mut data: [MaybeUninit<ArcSwapOption<T>>; CHUNK_SIZE] =
            MaybeUninit::uninit().assume_init();

        for elem in &mut data[..] {
            ptr::write(elem.as_mut_ptr(), ArcSwapOption::new(None));
        }

        mem::transmute(data)
    }
}
