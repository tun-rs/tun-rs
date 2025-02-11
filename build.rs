#[cfg(all(feature = "async_tokio", feature = "async_std", not(doc)))]
compile_error! {"More than one asynchronous runtime is simultaneously specified in features"}
fn main() {}
