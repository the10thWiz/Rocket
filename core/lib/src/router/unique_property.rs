use std::any::Any;
use std::fmt::{self, Display};

use super::Collide;

use crate::{Request, Route};
use crate::http::{uri::Host, MediaType};
use sealed::Sealed;

mod sealed {
    pub trait Sealed {}
}

pub trait AsAny: Any {
    /// Converts this object to a `dyn Any` type. Used as a polyfill
    /// for trait upcasting.
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait DynClone {
    fn clone_box(&self) -> Box<dyn UniqueProperty>;
}

impl<T: UniqueProperty + Clone> DynClone for T {
    fn clone_box(&self) -> Box<dyn UniqueProperty> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn UniqueProperty> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl fmt::Debug for dyn UniqueProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

pub trait UniqueProperty: AsAny + DynClone + Display + Sealed + Send + Sync {
    /// Checks whether `other` collides with self. If `other`
    /// does not check the same property, we should return None.
    ///
    /// This should be symmetrical, so `a.collides(b) == b.collides(a)`
    ///
    /// Two routes are considered colliding if there is not
    /// at least one property that returns `Some(false)`.
    fn collides(&self, self_route: &Route, other_route: &Route) -> Option<bool>;

    /// Checks whether a request matches this property.
    fn matches_request(&self, req: &Request<'_>) -> bool;
}

impl UniqueProperty for Host<'static> {
    fn collides(&self, _self_route: &Route, other_route: &Route) -> Option<bool> {
        other_route.get_unique_prop::<Self>().map(|other| other == self)
    }

    fn matches_request<'r>(&self, req: &'r Request<'_>) -> bool {
        match req.host() {
            Some(host) => self == host,
            None => false,
        }
    }
}
impl Sealed for Host<'static> {}

impl UniqueProperty for MediaType {
    fn collides(&self, self_route: &Route, other_route: &Route) -> Option<bool> {
        match (self_route.method.allows_request_body(), other_route.method.allows_request_body()) {
            (Some(true), Some(true)) => other_route.get_unique_prop().map(|other| self.collides_with(other)),
            _ => Some(true),
        }
    }

    fn matches_request<'r>(&self, req: &'r Request<'_>) -> bool {
        match req.method().allows_request_body() {
            Some(true) => match req.format() {
                Some(f) if f.specificity() == 2 => self.collides_with(f),
                _ => false
            },
            _ => match req.format() {
                Some(f) => self.collides_with(f),
                None => true
            }
        }
    }
}
impl Sealed for MediaType {}

pub(crate) fn dyn_box_any(b: &Box<dyn UniqueProperty>) -> &dyn Any {
    let b: &dyn UniqueProperty = b.as_ref();
    let any = b.as_any();
    assert_eq!(b.type_id(), any.type_id());
    any
    // <Box<dyn UniqueProperty> as AsRef<dyn UniqueProperty>>::as_ref(b).as_any()
    // todo!()
}

/// A set of properties is unambiguous iff there is at least one property shared by both sets, with
/// a different value.
pub(crate) fn collides(a: &Route, b: &Route) -> bool {
    for prop_a in &a.unique_properties {
        for prop_b in &b.unique_properties {
            // Check that they have the same type_id, which prevents checking other properties (and potentially
            // avoids the need to check the reverse)
            dbg!((prop_a.type_id(), prop_b.type_id(), prop_a.type_id() == prop_b.type_id()));
            dbg!(prop_a.collides(a, b));
            // `prop_b.as_ref()` is needed to ensure we do not convert `Box<dyn _>` into `&dyn Any`, but rather
            // get the inner type
            // assert_eq!(std::any::TypeId::of::<MediaType>(), dyn_box_any(prop_b).type_id());
            if dyn_box_any(prop_a).type_id() == dyn_box_any(prop_b).type_id() && prop_a.collides(a, b) == Some(false) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;
    use crate::http::uri::Host;
    use crate::uri;

    #[test]
    fn basic_api() {
        let host= Host::new(uri!("my.example.com"));
        let v: &dyn UniqueProperty = &host;
        assert_eq!(v.type_id(), TypeId::of::<Host<'static>>())
    }
}
