/// What `screenshot::take` returns: width, height, the encoded image, and
/// where the capture's top-left corner sits on the desktop, when it has one.
pub type Screenshot = (u32, u32, Vec<u8>, Option<(i32, i32)>);

#[cfg(all(windows, not(guictl_stub)))]
pub mod windows;
#[cfg(all(windows, not(guictl_stub)))]
pub use windows as os;

#[cfg(all(target_os = "linux", not(guictl_stub)))]
pub mod linux;
#[cfg(all(target_os = "linux", not(guictl_stub)))]
pub use linux as os;

// `--cfg guictl_stub` forces the stub on a supported platform, so it can be
// compile-checked without a third operating system at hand.
#[cfg(any(guictl_stub, not(any(windows, target_os = "linux"))))]
pub mod stub;
#[cfg(any(guictl_stub, not(any(windows, target_os = "linux"))))]
pub use stub as os;
