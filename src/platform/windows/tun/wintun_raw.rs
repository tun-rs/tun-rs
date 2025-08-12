#![allow(warnings)]
#[cfg(not(docsrs))]
#[cfg(feature = "build-bindings")]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(any(docsrs, not(feature = "build-bindings")))]
include!("bindings.rs");
