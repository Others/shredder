/// A trait implementing an alternative to `Drop`, useful for non-`GcDrop` data.
///
/// Usually when you have data in a `Gc` you just want its destructor to be called when the data is
/// collected. However, the collector can't naively run the `drop` method of non-`'static` data,
/// since it could access data with an elapsed lifetime. (It's even UB to create a reference into
/// a struct holding an invalid reference!) We address this in two parts. The `R` and `RMut` structs
/// provide a safe alternative to holding a direct reference with a non-'static lifetime. Then the
/// `Finalize` trait let's you opt-in to writing unsafe code at deallocation time.
///
/// Data that is `!GcDrop` for other reasons/contains a `DerefGc`
///
/// # Safety
/// When implementing this trait you're promising a few things:
///
/// 1) Tour data does not contain any non-`'static` references.
/// (You may use `R` and `RMut` instead!)
///
/// 2) Your `finalize` method does not access any data with a non-`'static` lifetime. In particular
/// you may not call any methods on `R` or `RMut` other than `finalize`. (No `Deref` either!)
///
/// 3) Your `finalize` method does not call any methods on a `AtomicGc` or `DerefGc`.
/// (No `Deref` either!)
///
/// 4) Your `finalize` method does not make an `AtomicGc`, `DerefGc`, `R` or `RMut` "live again."
/// Basically you must not send one of these pieces of data to another thread.
pub unsafe trait Finalize {
    /// Do cleanup on this data, potentially leaving it in an invalid state.
    /// (See trait documentation for the rules for implementing this method.)
    ///
    /// Please ensure your `finalize` implementations delegate properly and call your fields
    /// `finalize` methods after doing cleanup.
    ///
    /// # Safety
    /// After calling this method, no further operations may be performed with this object. You
    /// may not even drop this object! You must `mem::forget` it or otherwise force its destructor
    /// not to run.
    unsafe fn finalize(&mut self);
}
