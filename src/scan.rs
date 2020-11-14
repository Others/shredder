use crate::collector::InternalGcRef;
use crate::marker::GcSafe;

/// A trait capturing the ability of data to be scanned for references to data in a `Gc`.
///
/// This is unsafe, since a bad `scan` implementation can cause memory unsafety in two ways:
/// 1) If `scan` scans data that this object does not own
/// 2) If `scan` does anything other than `scan` data with a non-`'static` lifetime
/// 3) If `scan` is non-deterministic about what owned data it scans
///
/// The importance of (1) is so that the collector does not collect data that is in use. The
/// importance of (2) is so that data can still be scanned even after its lifetime has
/// technically expired.
///
/// Regarding (1): Note that it's okay to miss data that you own. Missing connected data can only
/// cause memory leaks--not memory unsafety.
/// Regarding (2): In particular, `scan` should not call anything but `Scan` on `R` and `RMut`. Even
/// implicitly using the `deref` implementations on these structs is incorrect.
///
/// Importantly, any empty `scan` implementation is safe (assuming the `GcSafe` impl is correct)
///
/// NB: It's important that `scan` only scans data that is truly owned. `Rc`/`Arc` cannot have
/// sensible `scan` implementations, since each individual smart pointer doesn't own the underlying
/// data.
///
/// # Examples
/// In practice you probably want to use the derive macro:
/// ```
/// use shredder::Scan;
///
/// #[derive(Scan)]
/// struct Example {
///     v: u32
/// }
/// ```
///
/// This also comes with a `#[shredder(skip_scan)]` attribute, for when some data implements
/// `GcSafe` but not `Scan`
/// ```
/// use std::sync::Arc;
/// use shredder::Scan;
///
/// #[derive(Scan)]
/// struct Example {
///     #[shredder(skip_scan)]
///     v: Arc<u32>
/// }
/// ```
///
/// This can work for any `Send+ 'static` data using `GcSafeWrapper`
/// ```
/// use std::sync::Arc;
///
/// use shredder::marker::GcSafeWrapper;
/// use shredder::Scan;
///
/// struct SendDataButNotScan {
///     i: u32
/// }
///
/// #[derive(Scan)]
/// #[shredder(cant_drop)] // <- To understand why we need this, read the docs of the derive itself
/// struct Example {
///     #[shredder(skip_scan)]
///     v: GcSafeWrapper<SendDataButNotScan>
/// }
/// ```
///
/// In emergencies, you can break out `#[shredder(unsafe_skip_gc_safe)]`, but this is potentially
/// unsafe (the field you're skipping MUST uphold invariants as-if it was `GcSafe`)
/// ```
/// use std::sync::Arc;
/// use shredder::Scan;
///
/// struct NotEvenSendData {
///     data: *mut u32
/// }
///
/// #[derive(Scan)]
/// #[shredder(cant_drop)] // <- To understand why we need this, read the docs of the derive itself
/// struct Example {
///     #[shredder(unsafe_skip_gc_safe)]
///     v: NotEvenSendData
/// }
/// ```
///
/// IMPORTANT NOTE: You may have problems with the derive complaining your data is not-`GcDrop`. To
/// find a resolution, read the documentation of the derive itself.
pub unsafe trait Scan: GcSafe {
    /// `scan` should use the scanner to scan all of its directly owned data
    fn scan(&self, scanner: &mut Scanner<'_>);
}

/// A trait that allows something that is `Scan` to be converted to a `dyn` ref.
///
/// Implementing this trait is only necessary if you need to allocate an owned pointer to a DST,
/// e.g. `Gc::from_box(Box<dyn MyTrait>)`
///
/// This is unsafe because `to_scan` must always be implemented as `&*self`
pub unsafe trait ToScan {
    /// Converts this value to a `dyn Scan` reference value.
    fn to_scan(&self) -> &(dyn Scan + 'static);
}

unsafe impl<T: Scan + Sized + 'static> ToScan for T {
    fn to_scan(&self) -> &(dyn Scan + 'static) {
        &*self
    }
}

/// Scanner is a struct used to manage the scanning of data, sort of analogous to `Hasher`
/// Usually you will only care about this while implementing `Scan`
pub struct Scanner<'a> {
    pub(crate) scan_callback: Box<dyn FnMut(InternalGcRef) + 'a>,
}

#[allow(clippy::unused_self)]
impl<'a> Scanner<'a> {
    #[must_use]
    pub(crate) fn new<F: FnMut(InternalGcRef) + 'a>(callback: F) -> Self {
        Self {
            scan_callback: Box::new(callback),
        }
    }

    /// Scan a piece of data, tracking any `Gc`s found
    #[inline]
    pub fn scan<T: Scan + ?Sized>(&mut self, from: &T) {
        from.scan(self);
    }

    #[inline]
    pub(crate) fn add_internal_handle(&mut self, gc_ref: InternalGcRef) {
        (self.scan_callback)(gc_ref);
    }
}
