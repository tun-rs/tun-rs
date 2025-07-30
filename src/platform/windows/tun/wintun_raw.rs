#![allow(warnings)]
#[cfg(not(doc))]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(doc)]
include!("bindings.rs");
