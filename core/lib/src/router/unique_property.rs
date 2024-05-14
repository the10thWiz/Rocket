use std::any::Any;

use crate::{request::{FromRequest, Outcome}, Request};



pub trait UniqueProperty: Any {
    fn as_any(&self) -> &dyn Any;
    
    fn matches(&self, other: &dyn Any) -> Option<bool>;

    // TODO: matches request. We want this so we can make routing decisions later, although we could choose to rely
    // on the handler to do this for us.
}

impl<T: Eq + Any> UniqueProperty for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn matches(&self, other: &dyn Any) -> Option<bool> {
        other.downcast_ref().map(|other: &Self| other == self)
    }
}

// pub struct UniqueInstance {
//     val: Arc<dyn Any>,
//     matcher: Box<dyn Fn(&dyn Any) -> Option<bool>>,
// }

// impl UniqueInstance {
//     pub fn new<T: UniqueProperty + 'static>(val: T) -> Self {
//         let arc = Arc::new(val);
//         Self {
//             val: arc.clone(),
//             matcher: Box::new(move |v| v.downcast_ref().map(|v: &T| v == arc.as_ref())),
//         }
//     }

//     /// Returns true iff `self` and `val` hold values of the same type, and have different
//     /// values.
//     pub fn prevents_collision(&self, val: &Self) -> bool {
//         (self.matcher)(val.val.as_ref()).map_or(false, |is_eq| !is_eq)
//     }
// }

// async fn matches_request<'a, 'r, T: UniqueProperty + Eq + 'a>(val: &dyn UniqueProperty, req: &'r Request<'_>) -> bool where &'a T: FromRequest<'r> {
//     match <&T as FromRequest>::from_request(req).await {
//         Outcome::Success(v) => val.matches(&v).map_or(false, |v| v),
//         _ => false,
//     }
// }


#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::uri::Host;
    use crate::uri;

    #[test]
    fn basic_api() {
        // let instance = UniqueInstance::new(Host::new(uri!("my.example.com")));
    }
}
