// desktop/mod.rs — Desktop-only modules gated by cfg(not(any(target_os = "android", target_os = "ios"))).

pub mod ffi;
pub mod model_manager;
pub mod automation;
pub mod system;
pub mod screen;
pub mod kb;
