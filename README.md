shredder
========
 `shredder` is a library providing a garbage collected smart pointer: `Gc`
 This is useful for times where you want an shared access to some data, but the structure
 of the data has unpredictable cycles in it. (So Arc would not be appropriate.)

 `shredder` has the following features
 - fairly ergonomic: no need to manually manage roots, just a regular smart pointer
 - destructors: no need for finalization, your destructors are seamlessly run
 - ready for fearless concurrency: works in multi-threaded contexts
 - safe: detects error conditions on the fly, and protects you from common mistakes
 - limited stop-the world: no regular processing on data can be interrupted
 - concurrent collection: collection and destruction happens in the background


 `shredder` has the following limitations
 - non-sync ergonomics: `Send` objects need a guard object
 - multiple collectors: multiple collectors do not co-operate
 - can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
 - no derive for `Scan`: this would make implementing `Scan` easier (WIP)
 - non static data: `Gc` cannot handle non 'static data (fix WIP)
