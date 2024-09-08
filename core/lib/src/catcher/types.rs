use either::Either;
use transient::{Any, CanRecoverFrom, CanTranscendTo, Downcast, Transience};
use crate::{http::Status, response::{self, Responder}, Request, Response};
#[doc(inline)]
pub use transient::{Static, Transient, TypeId, Inv};

/// Polyfill for trait upcasting to [`Any`]
pub trait AsAny<Tr: Transience>: Any<Tr> + Sealed {
    /// The actual upcast
    fn as_any(&self) -> &dyn Any<Tr>;
    /// convience typeid of the inner typeid
    fn trait_obj_typeid(&self) -> TypeId;
}

use sealed::Sealed;
mod sealed {
    use transient::{Any, Inv, Transient, TypeId};

    use super::AsAny;

    pub trait Sealed {}
    impl<'r, T: Any<Inv<'r>>> Sealed for T { }
    impl<'r, T: Any<Inv<'r>> + Transient> AsAny<Inv<'r>> for T {
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
pub trait TypedError<'r>: AsAny<Inv<'r>> + Send + Sync + 'r {
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
    fn source(&'r self) -> Option<&'r (dyn TypedError<'r> + 'r)> { None }

    /// Status code
    // TODO: This is currently only used for errors produced by Fairings
    fn status(&self) -> Status { Status::InternalServerError }
}

impl<'r> TypedError<'r> for std::convert::Infallible {  }

impl<'r> TypedError<'r> for () {  }

impl<'r> TypedError<'r> for std::io::Error {
    fn status(&self) -> Status {
        match self.kind() {
            std::io::ErrorKind::NotFound => Status::NotFound,
            _ => Status::InternalServerError,
        }
    }
}

impl<'r> TypedError<'r> for std::num::ParseIntError {}
impl<'r> TypedError<'r> for std::num::ParseFloatError {}
impl<'r> TypedError<'r> for std::string::FromUtf8Error {}

impl TypedError<'_> for Status {
    fn status(&self) -> Status { *self }
}

#[cfg(feature = "json")]
impl<'r> TypedError<'r> for serde_json::Error {}

#[cfg(feature = "msgpack")]
impl<'r> TypedError<'r> for rmp_serde::encode::Error {}
#[cfg(feature = "msgpack")]
impl<'r> TypedError<'r> for rmp_serde::decode::Error {}

// TODO: This is a hack to make any static type implement Transient
impl<'r, T: std::fmt::Debug + Send + Sync + 'static> TypedError<'r> for response::Debug<T> {
    fn respond_to(&self, request: &'r Request<'_>) -> Result<Response<'r>, Status> {
        format!("{:?}", self.0).respond_to(request).responder_error()
    }
}

impl<'r, L, R> TypedError<'r> for Either<L, R>
    where L: TypedError<'r> + Transient,
          L::Transience: CanTranscendTo<Inv<'r>>,
          R: TypedError<'r> + Transient,
          R::Transience: CanTranscendTo<Inv<'r>>,
{
    fn respond_to(&self, request: &'r Request<'_>) -> Result<Response<'r>, Status> {
        match self {
            Self::Left(v) => v.respond_to(request),
            Self::Right(v) => v.respond_to(request),
        }
    }

    fn name(&self) -> &'static str { std::any::type_name::<Self>() }

    fn source(&'r self) -> Option<&'r (dyn TypedError<'r> + 'r)> {
        match self {
            Self::Left(v) => Some(v),
            Self::Right(v) => Some(v),
        }
    }

    fn status(&self) -> Status {
        match self {
            Self::Left(v) => v.status(),
            Self::Right(v) => v.status(),
        }
    }
}

// TODO: This cannot be used as a bound on an untyped catcher to get any error type.
// This is mostly an implementation detail (and issue with double boxing) for
// the responder derive
#[derive(Transient)]
pub struct AnyError<'r>(pub Box<dyn TypedError<'r> + 'r>);

impl<'r> TypedError<'r> for AnyError<'r> {
    fn source(&'r self) -> Option<&'r (dyn TypedError<'r> + 'r)> {
        Some(self.0.as_ref())
    }
}

/// Validates that a type implements `TypedError`. Used by the `#[catch]` attribute to ensure
/// the `TypeError` is first in the diagnostics.
#[doc(hidden)]
pub fn type_id_of<'r, T: TypedError<'r> + Transient + 'r>() -> (TypeId, &'static str) {
    (TypeId::of::<T>(), std::any::type_name::<T>())
}

/// Downcast an error type to the underlying concrete type. Used by the `#[catch]` attribute.
#[doc(hidden)]
pub fn downcast<'r, T: TypedError<'r> + Transient + 'r>(v: Option<&'r dyn TypedError<'r>>) -> Option<&'r T>
    where T::Transience: CanRecoverFrom<Inv<'r>>
{
    // if v.is_none() {
    //     crate::trace::error!("No value to downcast from");
    // }
    let v = v?;
    // crate::trace::error!("Downcasting error from {}", v.name());
    v.as_any().downcast_ref()
}

/// Upcasts a value to `Box<dyn Error<'r>>`, falling back to a default if it doesn't implement
/// `Error`
#[doc(hidden)]
#[macro_export]
macro_rules! resolve_typed_catcher {
    ($T:expr) => ({
        #[allow(unused_imports)]
        use $crate::catcher::resolution::{Resolve, DefaultTypeErase, ResolvedTypedError};

        let inner = Resolve::new($T).cast();
        ResolvedTypedError {
            name: inner.as_ref().map(|e| e.name()),
            val: inner,
        }
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

        fn cast(self) -> Option<Box<dyn TypedError<'r>>> { None }
    }

    impl<'r, T: 'r> DefaultTypeErase<'r> for Resolve<'r, T> {}

    /// "Specialized" "implementation" of `Transient` for `T: Transient`. This is
    /// what Rust will resolve `Resolve<T>::item` to when `T: Transient`.
    impl<'r, T: TypedError<'r> + Transient> Resolve<'r, T>
        where T::Transience: CanTranscendTo<Inv<'r>>
    {
        pub const SPECIALIZED: bool = true;

        pub fn cast(self) -> Option<Box<dyn TypedError<'r>>> { Some(Box::new(self.0)) }
    }

    /// Wrapper type to hold the return type of `resolve_typed_catcher`.
    #[doc(hidden)]
    pub struct ResolvedTypedError<'r> {
        /// The return value from `TypedError::name()`, if Some
        pub name: Option<&'static str>,
        /// The upcast error, if it supports it
        pub val: Option<Box<dyn TypedError<'r> + 'r>>,
    }
}
