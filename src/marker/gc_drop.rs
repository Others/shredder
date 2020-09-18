/// A marker trait that the destructor of this data can be safely run in the background thread
///
/// Basically it asserts three things
/// 1) Any thread can drop this data (It is `Send`, or `!Send` purely because it contains a `Gc`, or
/// it is `!Send` but you know that any thread dropping this data is safe.)
/// 2) This data does not own a `AtomicGc` or `DerefGc`
/// 3) This data is `'static`, or you can guarantee that it's safe to drop it after its lifetime
/// has ended.
///
/// This trait is structural, although I'm sure with some crazy unsafe code you could break that
/// assumption.
///
/// The `Scan` derive will automatically generate a `GcDrop` impl for you. For example:
/// ```
/// use shredder::Scan;
///
/// // SomeGcDropType will implement `GcDrop` as well as `Scan` and `GcSafe`
/// #[derive(Scan)]
/// struct SomeGcDropType {
///     random_data: u32
/// }
/// ```
///
/// If this is causing you problems you can turn it off with the `#[shredder(cant_drop)]`
/// annotation. For example:
/// ```
/// use shredder::atomic::AtomicGc;
/// use shredder::Scan;
///
/// // CantImplGcDrop will not get a `GcDrop` implementation
/// #[derive(Scan)]
/// #[shredder(cant_drop)] // <- This allows this code to compile
/// struct CantImplGcDrop {
///     some_non_gc_drop_type: AtomicGc<u32>
/// }
/// ```
pub unsafe trait GcDrop {}
