use std::cell::{BorrowError, BorrowMutError, RefCell};
use std::ops::{Deref, DerefMut};

use crate::{Gc, Scan};

// This is special casing for Gc<RefCell<T>>
rental! {
    mod gc_refcell_internals {
        use crate::{Scan, GcGuard};
        use std::cell::{Ref, RefCell, RefMut};

        /// Self referential wrapper around `Ref` for ergonomics
        #[rental(deref_suffix)]
        pub struct GcRefInt<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RefCell<T>>,
            cell_ref: Ref<'gc_guard, T>
        }

        /// Self referential wrapper around `RefMut` for ergonomics
        #[rental(deref_mut_suffix)]
        pub struct GcRefMutInt<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RefCell<T>>,
            cell_ref: RefMut<'gc_guard, T>
        }
    }
}

/// This is like a `Ref`, but taken directly from a `Gc`
pub struct GcRef<'a, T: Scan + 'static> {
    internal_ref: gc_refcell_internals::GcRefInt<'a, T>,
}

impl<'a, T: Scan + 'static> Deref for GcRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_ref.deref()
    }
}

/// This is like a `RefMut`, but taken directly from a `Gc`
pub struct GcRefMut<'a, T: Scan + 'static> {
    internal_ref: gc_refcell_internals::GcRefMutInt<'a, T>,
}

impl<'a, T: Scan + 'static> Deref for GcRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_ref.deref()
    }
}

impl<'a, T: Scan + 'static> DerefMut for GcRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.internal_ref.deref_mut()
    }
}

impl<T: Scan + 'static> Gc<RefCell<T>> {
    /// Call the underlying `borrow` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    #[must_use]
    pub fn borrow(&self) -> GcRef<'_, T> {
        let g = self.get();
        let internal_ref = gc_refcell_internals::GcRefInt::new(g, RefCell::borrow);

        GcRef { internal_ref }
    }

    /// Call the underlying `try_borrow` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    ///
    /// # Errors
    /// Propagates a `BorrowError` if the underlying `RefCell` is already borrowed mutably
    pub fn try_borrow(&self) -> Result<GcRef<'_, T>, BorrowError> {
        let g = self.get();
        let internal_ref =
            gc_refcell_internals::GcRefInt::try_new(g, RefCell::try_borrow).map_err(|e| e.0)?;

        Ok(GcRef { internal_ref })
    }

    /// Call the underlying `borrow_mut` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    #[must_use]
    pub fn borrow_mut(&self) -> GcRefMut<'_, T> {
        let g = self.get();
        let internal_ref = gc_refcell_internals::GcRefMutInt::new(g, RefCell::borrow_mut);

        GcRefMut { internal_ref }
    }

    /// Call the underlying `try_borrow_mut` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    /// # Errors
    /// Propagates a `BorrowError` if the underlying `RefCell` is already borrowed
    pub fn try_borrow_mut(&self) -> Result<GcRefMut<'_, T>, BorrowMutError> {
        let g = self.get();
        let internal_ref = gc_refcell_internals::GcRefMutInt::try_new(g, RefCell::try_borrow_mut)
            .map_err(|e| e.0)?;

        Ok(GcRefMut { internal_ref })
    }
}
