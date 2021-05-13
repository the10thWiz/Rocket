
/// A trait for channel descriptors to implement
///
/// There should eventually be a derive macro or
/// some kind of better option. Potentially a macro
/// to generate channels.
pub trait ChannelDescriptor {
    /// Checks whether a message sent to the channel
    /// descriptor `other` should be sent to a channel
    /// subscibed to `self`.
    fn matches(&self, other: &Self) -> bool;
}


