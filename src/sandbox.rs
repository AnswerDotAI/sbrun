#[cfg(target_os = "macos")]
#[path = "sandbox_macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "sandbox_linux.rs"]
mod platform;

pub use platform::apply;
