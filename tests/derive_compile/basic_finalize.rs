use shredder::Finalize;

struct NotFinalize;

#[derive(Finalize)]
struct Test0 {}

#[derive(Finalize)]
struct Test1();

#[derive(Finalize)]
struct Test2 {
    i: u32,
}

#[derive(Finalize)]
struct Test3 {
    i: u32,
    j: u32,
    k: u32,
}

#[derive(Finalize)]
struct Test4(u32, u32, u32);

#[derive(Finalize)]
struct Test5 {
    i: u32,
    j: u32,
    #[shredder(skip_finalize)]
    k: NotFinalize,
}

#[derive(Finalize)]
struct Test6 {
    i: u32,
    j: u32,
    #[shredder(unsafe_skip_all)]
    k: NotFinalize,
}

#[derive(Finalize)]
enum Test7 {

}

#[derive(Finalize)]
enum Test8 {
    A(u32),
    B(bool)
}

#[derive(Finalize)]
enum Test9 {
    A {
        a: u32,
        #[shredder(skip_finalize)]
        b: NotFinalize
    },
    B(bool)
}

#[derive(Finalize)]
enum Test10 {
    A(#[shredder(skip_finalize)] NotFinalize)
}

fn main() {}
