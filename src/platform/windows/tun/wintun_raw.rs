#![allow(warnings)]

#[cfg(all(not(docsrs), feature = "bindgen"))]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(any(docsrs, not(feature = "bindgen")))]
#[cfg(target_pointer_width = "64")]
include!("bindings_x86_64.rs");

#[cfg(any(docsrs, not(feature = "bindgen")))]
#[cfg(target_pointer_width = "32")]
include!("bindings_i686.rs");
