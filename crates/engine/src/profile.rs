//! CPU profiling zones, compiled to nothing unless the `tracy` feature is on.
//!
//! `zone!("name")` opens a zone that closes at the end of the enclosing scope.

#[cfg(feature = "tracy")]
pub use tracy_client;

#[cfg(feature = "tracy")]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {
        let _zone = $crate::profile::tracy_client::span!($name);
    };
}

#[cfg(not(feature = "tracy"))]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {};
}

/// Ends the current frame in the Tracy timeline.
pub fn frame_mark() {
    #[cfg(feature = "tracy")]
    tracy_client::frame_mark();
}
