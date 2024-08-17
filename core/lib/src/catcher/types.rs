use either::Either;
use transient::{Any, CanRecoverFrom, Inv, Downcast, Transience};
use crate::{http::Status, Request, Response};
#[doc(inline)]
pub use transient::{Static, Transient, TypeId};

/// Polyfill for trait upcasting to [`Any`]
pub trait AsAny<Tr: Transience>: Any<Tr> + Sealed {
    /// The actual upcast
    fn as_any(&self) -> &dyn Any<Tr>;
    /// convience typeid of the inner typeid
    fn trait_obj_typeid(&self) -> TypeId;
}

use sealed::Sealed;
mod sealed {
    use transient::{Any, Inv, TypeId};

    use super::AsAny;

    pub trait Sealed {}
    impl<'r, T: Any<Inv<'r>>> Sealed for T { }
    impl<'r, T: Any<Inv<'r>>> AsAny<Inv<'r>> for T {
        fn as_any(&self) -> &dyn Any<Inv<'r>> {
            self
        }
        fn trait_obj_typeid(&self) -> transient::TypeId {
            TypeId::of::<T>()
        }
    }
}

/// This is the core of typed catchers. If an error type (returned by
/// FromParam, FromRequest, FromForm, FromData, or Responder) implements
/// this trait, it can be caught by a typed catcher. (TODO) This trait
/// can be derived.
pub trait Error<'r>: AsAny<Inv<'r>> + Send + Sync + 'r {
    /// Generates a default response for this type (or forwards to a default catcher)
    #[allow(unused_variables)]
    fn respond_to(&self, request: &'r Request<'_>) -> Result<Response<'r>, Status> {
        Err(Status::InternalServerError)
    }
    
    /// A descriptive name of this error type. Defaults to the type name.
    fn name(&self) -> &'static str { std::any::type_name::<Self>() }

    /// The error that caused this error. Defaults to None.
    ///
    /// # Warning
    /// A typed catcher will not attempt to follow the source of an error
    /// more than once.
    fn source(&self) -> Option<&dyn Error<'r>> { None }

    /// Status code
    fn status(&self) -> Status { Status::InternalServerError }
}

impl<'r> Error<'r> for std::convert::Infallible {  }
impl<'r, L: Error<'r>, R: Error<'r>> Error<'r> for Either<L, R> {
    fn respond_to(&self, request: &'r Request<'_>) -> Result<Response<'r>, Status> {
        match self {
            Self::Left(v) => v.respond_to(request),
            Self::Right(v) => v.respond_to(request),
        }
    }

    fn name(&self) -> &'static str { std::any::type_name::<Self>() }

    fn source(&self) -> Option<&dyn Error<'r>> { 
        match self {
            Self::Left(v) => v.source(),
            Self::Right(v) => v.source(),
        }
    }

    fn status(&self) -> Status {
        match self {
            Self::Left(v) => v.status(),
            Self::Right(v) => v.status(),
        }
    }
}

pub fn downcast<'a, 'r, T: Transient + 'r>(v: &'a dyn Error<'r>) -> Option<&'a T>
    where T::Transience: CanRecoverFrom<Inv<'r>>
{
    v.as_any().downcast_ref()
}

/// Upcasts a value to `Box<dyn Error<'r>>`, falling back to a default if it doesn't implement
/// `Error`
#[doc(hidden)]
#[macro_export]
macro_rules! resolve_typed_catcher {
    ($T:expr) => ({
        #[allow(unused_imports)]
        use $crate::catcher::resolution::{Resolve, DefaultTypeErase};

        Resolve::new($T).cast()
    });
}

pub use resolve_typed_catcher;

pub mod resolution {
    use std::marker::PhantomData;

    use transient::{CanTranscendTo, Transient};

    use super::*;

    /// The *magic*.
    ///
    /// `Resolve<T>::item` for `T: Transient` is `<T as Transient>::item`.
    /// `Resolve<T>::item` for `T: !Transient` is `DefaultTypeErase::item`.
    ///
    /// This _must_ be used as `Resolve::<T>:item` for resolution to work. This
    /// is a fun, static dispatch hack for "specialization" that works because
    /// Rust prefers inherent methods over blanket trait impl methods.
    pub struct Resolve<'r, T: 'r>(T, PhantomData<&'r ()>);

    impl<'r, T: 'r> Resolve<'r, T> {
        pub fn new(val: T) -> Self {
            Self(val, PhantomData)
        }
    }

    /// Fallback trait "implementing" `Transient` for all types. This is what
    /// Rust will resolve `Resolve<T>::item` to when `T: !Transient`.
    pub trait DefaultTypeErase<'r>: Sized {
        const SPECIALIZED: bool = false;

        fn cast(self) -> Option<Box<dyn Error<'r>>> { None }
    }

    impl<'r, T: 'r> DefaultTypeErase<'r> for Resolve<'r, T> {}

    /// "Specialized" "implementation" of `Transient` for `T: Transient`. This is
    /// what Rust will resolve `Resolve<T>::item` to when `T: Transient`.
    impl<'r, T: Error<'r> + Transient> Resolve<'r, T>
        where T::Transience: CanTranscendTo<Inv<'r>>
    {
        pub const SPECIALIZED: bool = true;

        pub fn cast(self) -> Option<Box<dyn Error<'r>>> { Some(Box::new(self.0))}
    }
}

// #[cfg(test)]
// mod test {
//     // use std::any::TypeId;

//     use transient::{Transient, TypeId};

//     use super::resolution::{Resolve, DefaultTypeErase};

//     struct NotAny;
//     #[derive(Transient)]
//     struct YesAny;

//     // #[test]
//     // fn check_can_determine() {
//     //     let not_any = Resolve::new(NotAny).cast();
//     //     assert_eq!(not_any.type_id(), TypeId::of::<()>());

//     //     let yes_any = Resolve::new(YesAny).cast();
//     //     assert_ne!(yes_any.type_id(), TypeId::of::<()>());
//     // }

//     // struct HasSentinel<T>(T);

//     // #[test]
//     // fn parent_works() {
//     //     let child = resolve!(YesASentinel, HasSentinel<YesASentinel>);
//     //     assert!(child.type_name.ends_with("YesASentinel"));
//     //     assert_eq!(child.parent.unwrap(), TypeId::of::<HasSentinel<YesASentinel>>());
//     //     assert!(child.specialized);

//     //     let not_a_direct_sentinel = resolve!(HasSentinel<YesASentinel>);
//     //     assert!(not_a_direct_sentinel.type_name.contains("HasSentinel"));
//     //     assert!(not_a_direct_sentinel.type_name.contains("YesASentinel"));
//     //     assert!(not_a_direct_sentinel.parent.is_none());
//     //     assert!(!not_a_direct_sentinel.specialized);
//     // }
// }
