#[macro_use] extern crate rocket;

#[derive(TypedError)]
struct InnerError;
struct InnerNonError;

#[derive(TypedError)]
struct Thing1<'a, 'b> {
    a: &'a str,
    b: &'b str,
}

#[derive(TypedError)]
struct Thing2 {
    #[error(source)]
    inner: InnerNonError,
}

#[derive(TypedError)]
enum Thing3<'a, 'b> {
    A(&'a str),
    B(&'b str),
}

#[derive(TypedError)]
enum Thing4 {
    A(#[error(source)] InnerNonError),
    B(#[error(source)] InnerError),
}

#[derive(TypedError)]
enum EmptyEnum { }
