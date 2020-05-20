use std::alloc::{alloc, dealloc, Layout};
use std::mem::ManuallyDrop;
use std::panic::UnwindSafe;
use std::ptr;

use crate::collector::InternalGcRef;
use crate::{Scan, Scanner};

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcAllocation(*const dyn Scan);

// We need this for the drop thread. By that point we have exclusive access to the data
// It also, by contract of Scan, cannot have a Drop method that is unsafe in any thead
unsafe impl Send for GcAllocation {}
// Therefore, GcDataPtr is also UnwindSafe in the context we need it to be
impl UnwindSafe for GcAllocation {}
// We use the lockout to ensure that `GcDataPtr`s are not shared
unsafe impl Sync for GcAllocation {}

impl GcAllocation {
    pub fn allocate<T: Scan + 'static>(v: T) -> (Self, *const T) {
        // This is a straightforward use of alloc/write -- it should be undef free
        let data_ptr = unsafe {
            let heap_space = alloc(Layout::new::<T>()) as *mut T;
            ptr::write(heap_space, v);
            // NOTE: Write moves the data into the heap

            // Heap space is now a pointer to a T
            heap_space as *const T
        };

        let fat_ptr: *const dyn Scan = data_ptr;

        (Self(fat_ptr), data_ptr)
    }

    // This is unsafe, since we must externally guarantee that no-one still holds a pointer to the data
    // (Luckily this is the point of the garbage collector!)
    pub unsafe fn deallocate(self) {
        let scan_ptr: *const dyn Scan = self.0;

        // This calls the destructor of the Scan data
        {
            // Safe type shift: the contract of this method is that the scan_ptr doesn't alias
            // + ManuallyDrop is repr(transparent)
            let droppable_ptr: *mut ManuallyDrop<dyn Scan> =
                scan_ptr as *mut ManuallyDrop<dyn Scan>;
            let droppable_ref = &mut *droppable_ptr;
            ManuallyDrop::drop(droppable_ref);
        }

        let dealloc_layout = Layout::for_value(&*scan_ptr);
        let heap_ptr = scan_ptr as *mut u8;
        dealloc(heap_ptr, dealloc_layout);
    }

    pub fn scan<F: FnMut(InternalGcRef)>(&self, callback: F) {
        unsafe {
            let mut scanner = Scanner::new(callback);
            let to_scan = &*self.0;
            to_scan.scan(&mut scanner);
        }
    }

    #[cfg(test)]
    pub(crate) unsafe fn raw(v: *const dyn Scan) -> GcAllocation {
        GcAllocation(v)
    }
}
