use std::alloc::{alloc, dealloc, Layout};
use std::mem::{self, ManuallyDrop};
use std::panic::UnwindSafe;
use std::ptr;

use crate::collector::InternalGcRef;
use crate::{Finalize, Scan, Scanner, ToScan};

/// Represents a piece of data allocated by shredder
#[derive(Copy, Clone, Debug, Hash)]
pub struct GcAllocation {
    scan_ptr: *const dyn Scan,
    deallocation_action: DeallocationAction,
}

/// What additional action should we run before deallocating?
#[derive(Copy, Clone, Debug, Hash)]
pub enum DeallocationAction {
    BoxDrop,
    DoNothing,
    RunDrop,
    RunFinalizer { finalize_ptr: *const dyn Finalize },
}

// We need this for the drop thread. By that point we have exclusive access to the data
// It also, by contract of Scan, cannot have a Drop method that is unsafe in any thead
unsafe impl Send for GcAllocation {}
// Therefore, GcDataPtr is also UnwindSafe in the context we need it to be
impl UnwindSafe for GcAllocation {}
// We use the lockout to ensure that `GcDataPtr`s are not shared
unsafe impl Sync for GcAllocation {}

impl GcAllocation {
    pub fn allocate_with_drop<T: Scan + 'static>(v: T) -> (Self, *const T) {
        let (scan_ptr, raw_ptr) = Self::raw_allocate(v);
        (
            Self {
                scan_ptr,
                deallocation_action: DeallocationAction::RunDrop,
            },
            raw_ptr,
        )
    }

    pub fn allocate_no_drop<T: Scan>(v: T) -> (Self, *const T) {
        let (scan_ptr, raw_ptr) = Self::raw_allocate(v);
        (
            Self {
                scan_ptr,
                deallocation_action: DeallocationAction::DoNothing,
            },
            raw_ptr,
        )
    }

    #[allow(clippy::transmute_ptr_to_ptr)]
    pub fn allocate_with_finalization<T: Scan + Finalize>(v: T) -> (Self, *const T) {
        let (scan_ptr, raw_ptr) = Self::raw_allocate(v);

        let finalize_ptr = unsafe { mem::transmute(raw_ptr as *const dyn Finalize) };

        (
            Self {
                scan_ptr,
                deallocation_action: DeallocationAction::RunFinalizer { finalize_ptr },
            },
            raw_ptr,
        )
    }

    pub fn from_box<T: Scan + ToScan + ?Sized + 'static>(v: Box<T>) -> (Self, *const T) {
        let scan_ptr: *const dyn Scan = v.to_scan();
        let raw_ptr: *const T = Box::into_raw(v);

        (
            Self {
                scan_ptr,
                deallocation_action: DeallocationAction::BoxDrop,
            },
            raw_ptr,
        )
    }

    #[allow(clippy::transmute_ptr_to_ptr)]
    fn raw_allocate<'a, T: Scan + 'a>(v: T) -> (*const dyn Scan, *const T) {
        // This is a straightforward use of alloc/write -- it should be undef free
        let data_ptr = unsafe {
            let heap_space = alloc(Layout::new::<T>()) as *mut T;
            ptr::write(heap_space, v);
            // NOTE: Write moves the data into the heap

            // Heap space is now a pointer to a T
            heap_space as *const T
        };

        let fat_ptr: *const (dyn Scan + 'a) = data_ptr;
        // The contract of `Scan` ensures the `scan` method can be called after lifetimes end
        let fat_ptr: *const dyn Scan = unsafe { mem::transmute(fat_ptr) };

        (fat_ptr, data_ptr)
    }

    // This is unsafe, since we must externally guarantee that no-one still holds a pointer to the data
    // (Luckily this is the point of the garbage collector!)
    pub unsafe fn deallocate(self) {
        let scan_ptr: *const dyn Scan = self.scan_ptr;

        match self.deallocation_action {
            DeallocationAction::DoNothing => {
                // The name here is a bit of a lie, because we still need to invalidate handles
                let mut scanner = Scanner::new(|h| {
                    h.invalidate();
                });
                (&*scan_ptr).scan(&mut scanner);
            }
            DeallocationAction::RunDrop => {
                // Safe type shift: the contract of this method is that the scan_ptr doesn't alias
                // + ManuallyDrop is repr(transparent)
                let droppable_ptr = scan_ptr as *mut ManuallyDrop<dyn Scan>;
                let droppable_ref = &mut *droppable_ptr;
                ManuallyDrop::drop(droppable_ref);
            }
            DeallocationAction::RunFinalizer { finalize_ptr } => {
                // First of all invalidate handles, just in case of a bad `Finalize` implementation
                // (If it doesn't delegate correctly, `Gc`s could be left dangling)
                {
                    let mut scanner = Scanner::new(|h| {
                        h.invalidate();
                    });
                    (&*scan_ptr).scan(&mut scanner);
                }

                // We know this method can only be called if `scan_ptr` doesn't alias
                // And we know `finalize_ptr` ~= `scan_ptr`
                // So we can run `finalize` here, right before deallocation
                (&mut *(finalize_ptr as *mut dyn Finalize)).finalize();
            }
            DeallocationAction::BoxDrop => {
                // Safe as long as only boxed values are created with BoxDrop deallocate action
                // Additionally, this is the only instance where the pointer should be alive so
                // we are not taking it mutably anywhere else. The death of a pointer in action,
                // really makes you think...
                let box_ptr = Box::from_raw(scan_ptr as *mut dyn Scan);
                // drop like normal
                drop(box_ptr);
            }
        }

        // Only call dealloc() if we're not dealing with a boxed value, because the box gets
        // dropped above.
        if !matches!(self.deallocation_action, DeallocationAction::BoxDrop) {
            let dealloc_layout = Layout::for_value(&*scan_ptr);
            let heap_ptr = scan_ptr as *mut u8;
            dealloc(heap_ptr, dealloc_layout);
        }
    }

    pub fn scan<F: FnMut(InternalGcRef)>(&self, callback: F) {
        unsafe {
            let mut scanner = Scanner::new(callback);
            let to_scan = &*self.scan_ptr;
            to_scan.scan(&mut scanner);
        }
    }

    #[cfg(test)]
    pub(crate) unsafe fn raw(v: *const dyn Scan) -> GcAllocation {
        GcAllocation {
            scan_ptr: v,
            deallocation_action: DeallocationAction::DoNothing,
        }
    }
}
