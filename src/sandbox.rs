#[cfg(target_os = "macos")]
#[path = "sandbox_macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "sandbox_linux.rs"]
mod platform;

pub use platform::apply;
#[cfg(target_os = "linux")]
pub use platform::{PrivilegeMode, mode_from_ids, needs_preflight_drop, temporarily_drop_to_real_user, tmp_overlaps_writes};
