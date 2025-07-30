#![allow(warnings)]
#[cfg(not(docsrs))]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(docsrs)]
include!("bindings.rs");
