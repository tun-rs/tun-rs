#![allow(warnings)]
#[cfg(not(docsrs))]
#[cfg(feature = "bindgen")]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(any(docsrs, not(feature = "bindgen")))]
include!("bindings.rs");
