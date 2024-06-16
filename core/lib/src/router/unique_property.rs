use std::any::Any;
use std::fmt;

use super::Collide;

use crate::{Request, Route};
use crate::http::{uri::Host, MediaType};
use figment::value::UncasedStr;
use sealed::Sealed;

mod sealed {
    pub trait Sealed {}
}

pub trait AsAny: Any {
    /// Converts this object to a `dyn Any` type. Used as a polyfill
    /// for trait upcasting.
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Sealed> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl<T: Any> Sealed for T {}

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

pub trait UniqueProperty: AsAny + DynClone + fmt::Display + Send + Sync {
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

#[derive(Debug, Clone)]
pub enum WildcardHost {
    Wildcard(Host<'static>),
    Full(Host<'static>),
}

impl WildcardHost {
    pub fn parse(s: impl Into<String>) -> Result<Self, crate::http::uri::Error<'static>> {
        let s = s.into();
        if let Some(host) = s.strip_prefix("*.") {
            Host::parse_owned(host.into()).map(Self::Wildcard)
        } else {
            Host::parse_owned(s).map(Self::Full)
        }
    }
}

fn is_prefix(longer: &Host<'_>, shorter: &Host<'_>) -> bool {
    let longer: &UncasedStr = longer.domain();
    let shorter: &UncasedStr = shorter.domain();
    // Hack for `ends_with`. This should be added to `uncased`
    &longer[longer.len().saturating_sub(shorter.len())..] == shorter
}

impl UniqueProperty for WildcardHost {
    fn collides(&self, _self_route: &Route, other_route: &Route) -> Option<bool> {
        other_route.get_unique_prop::<Self>().map(|other| match (self, other) {
            (Self::Full(t), Self::Full(o)) => t == o,
            (Self::Wildcard(t), Self::Wildcard(o))
                => !is_prefix(t, o) && !is_prefix(o, t),
            (Self::Full(t), Self::Wildcard(o))
                => !is_prefix(t, o),
            (Self::Wildcard(t), Self::Full(o))
                => !is_prefix(o, t),
        })
    }

    fn matches_request<'r>(&self, req: &'r Request<'_>) -> bool {
        match req.host() {
            Some(host) => match self {
                Self::Full(h) => h == host,
                Self::Wildcard(h) => is_prefix(host, h),
            },
            None => false,
        }
    }
}

impl fmt::Display for WildcardHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wildcard(h) => write!(f, "*.{}", h),
            Self::Full(h) => write!(f, "{}", h),
        }
    }
}

// impl UniqueProperty for Host<'static> {
//     fn collides(&self, _self_route: &Route, other_route: &Route) -> Option<bool> {
//         other_route.get_unique_prop::<Self>().map(|other| other == self)
//     }

//     fn matches_request<'r>(&self, req: &'r Request<'_>) -> bool {
//         match req.host() {
//             Some(host) => self == host,
//             None => false,
//         }
//     }
// }

impl UniqueProperty for MediaType {
    fn collides(&self, self_route: &Route, other_route: &Route) -> Option<bool> {
        // TODO: do we NEED to check methods here?
        match (self_route.method.allows_request_body(), other_route.method.allows_request_body()) {
            (Some(true), Some(true)) => other_route
                .get_unique_prop()
                // TODO: move the collides_with impl to this module
                .map(|other| self.collides_with(other)),
            _ => None, // Does not differentiate routes
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

pub(crate) fn dyn_box_any(b: &Box<dyn UniqueProperty>) -> &dyn Any {
    let b: &dyn UniqueProperty = b.as_ref();
    let any = b.as_any();
    // assert_eq!(b.type_id(), any.type_id());
    any
}

/// A set of properties is unambiguous iff there is at least one property shared by both sets, with
/// a different value.
pub(crate) fn collides(a: &Route, b: &Route) -> bool {
    for prop_a in &a.unique_properties {
        if prop_a.collides(a, b) == Some(false) {
            return false;
        }
    }
    // Check inverse. This shouldn't be needed, but I've included it for completeness
    // This check is only done once (during launch), so processing time doesn't really
    // matter
    for prop_b in &b.unique_properties {
        if prop_b.collides(b, a) == Some(false) {
            return false;
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
        let host= WildcardHost::Full(Host::new(uri!("my.example.com")));
        let v: &dyn UniqueProperty = &host;
        assert_eq!(v.type_id(), TypeId::of::<WildcardHost>())
    }
}
