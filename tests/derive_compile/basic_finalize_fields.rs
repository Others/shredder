use shredder::FinalizeFields;

struct NotFinalize;

#[derive(FinalizeFields)]
struct Test0 {}

#[derive(FinalizeFields)]
struct Test1();

#[derive(FinalizeFields)]
struct Test2 {
    i: u32,
}

#[derive(FinalizeFields)]
struct Test3 {
    i: u32,
    j: u32,
    k: u32,
}

#[derive(FinalizeFields)]
struct Test4(u32, u32, u32);

#[derive(FinalizeFields)]
struct Test5 {
    i: u32,
    j: u32,
    #[shredder(skip_finalize)]
    k: NotFinalize,
}

#[derive(FinalizeFields)]
struct Test6 {
    i: u32,
    j: u32,
    #[shredder(unsafe_skip_all)]
    k: NotFinalize,
}

#[derive(FinalizeFields)]
enum Test7 {

}

#[derive(FinalizeFields)]
enum Test8 {
    A(u32),
    B(bool)
}

#[derive(FinalizeFields)]
enum Test9 {
    A {
        a: u32,
        #[shredder(skip_finalize)]
        b: NotFinalize
    },
    B(bool)
}

#[derive(FinalizeFields)]
enum Test10 {
    A(#[shredder(skip_finalize)] NotFinalize)
}




fn main() {}
