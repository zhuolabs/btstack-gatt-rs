//! Failure-path checks runnable on Linux or an Android test environment.
#![cfg(any(target_os = "android", target_os = "linux"))]

use btstack_nusb::NusbHciTransport;
use std::{fs::File, os::fd::AsFd};

#[test]
fn rejects_non_usb_owned_fd() {
    let file = File::open("/dev/null").unwrap();
    assert!(NusbHciTransport::from_fd(file.into()).is_err());
}

#[test]
fn borrowed_fd_is_not_closed_when_usb_initialization_fails() {
    let file = File::open("/dev/null").unwrap();
    assert!(NusbHciTransport::from_borrowed_fd(file.as_fd()).is_err());
    // fstat must still succeed: the library must only close its own duplicate.
    file.metadata()
        .expect("caller's FD was unexpectedly closed");
}

#[cfg(target_os = "android")]
#[test]
fn desktop_open_returns_error_instead_of_enumerating_android() {
    assert!(NusbHciTransport::open(btstack_nusb::UsbDeviceSelector::new(0x0411, 0x0374)).is_err());
}
