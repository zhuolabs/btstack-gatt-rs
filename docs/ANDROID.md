# Android USB integration

Android support uses the same BTstack thread, GATT API, and nusb HCI transport as
the desktop implementation. The difference is how the USB device is opened:

```text
UsbManager permission -> openDevice -> UsbDeviceConnection file descriptor
    -> owned duplicate -> nusb::Device::from_fd -> NusbHciTransport -> GattServer
```

No nusb enumeration or reopening `/dev/bus/usb` by path is performed. Android's
`NusbHciTransport::open(UsbDeviceSelector)` returns an explanatory error: select
the device in the Android application and pass its FD instead.

## Android application responsibilities

1. Declare the USB host feature in the manifest and use a device with USB host/OTG support.
2. Select the desired `UsbDevice`, for example by `vendorId` and `productId`.
3. Check `UsbManager.hasPermission(device)`. If needed, request permission and
   wait for the permission result before opening the device.
4. Call `UsbManager.openDevice(device)` and obtain the connection's file descriptor.
5. Pass it to your Rust/JNI layer on a worker thread. Native startup blocks until
   the controller confirms advertising or startup fails.

Manifest entry:

```xml
<uses-feature android:name="android.hardware.usb.host" android:required="true" />
```

Selecting and opening a device after permission has been granted:

```kotlin
val device = usbManager.deviceList.values.single {
    it.vendorId == 0x0411 && it.productId == 0x0374
}
check(usbManager.hasPermission(device))
val connection = checkNotNull(usbManager.openDevice(device))
val rawFd = connection.fileDescriptor
check(rawFd >= 0)
// Pass rawFd to your native layer while connection remains open.
// Do not call connection.claimInterface(): nusb claims interface 0.
```

This is library integration guidance, not a complete Android app. Permission UI,
the JNI boundary, lifecycle/foreground-service policy, and application event
handling belong to the integrating app. Refer to the official
[USB host guide](https://developer.android.com/develop/connectivity/usb/host) and
[`getFileDescriptor()` documentation](https://developer.android.com/reference/android/hardware/usb/UsbDeviceConnection#getFileDescriptor()).

## Rust: take ownership of an FD

Use this when the native layer already owns a usbdevfs FD or a duplicate whose
ownership has been explicitly transferred to Rust:

```rust
use std::os::fd::OwnedFd;
use btstack_gatt::{Error, GattCharacteristic, GattServer, GattService};
use btstack_nusb::NusbHciTransport;

fn start_server(fd: OwnedFd) -> Result<GattServer, Error> {
    let transport = NusbHciTransport::from_fd(fd)?;
    GattServer::builder(transport)
        .name("Rust GATT")
        .service(
            GattService::new([
                0x6e, 0x40, 0, 1, 0xb5, 0xa3, 0xf3, 0x93,
                0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
            ])
            .characteristic(
                GattCharacteristic::new([
                    0x6e, 0x40, 0, 2, 0xb5, 0xa3, 0xf3, 0x93,
                    0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
                ])
                .write()
                .write_without_response()
                .on_write(|request| {
                    println!("RX: {:02x?}", request.value());
                    Ok(())
                }),
            ),
        )
        .start()
}
```

The FD is owned by nusb after the call. It is released on initialization failure
or when the transport is dropped. If opening through `from_device`, other nusb
clones held by the caller can keep the underlying device alive.

## Rust: borrow Android's FD without taking ownership

Android retains ownership of the FD returned by `getFileDescriptor()`. Do not
construct `OwnedFd::from_raw_fd(raw_fd)` directly from that borrowed integer.
Duplicate it first. The library's `from_borrowed_fd` method does that for you:

```rust
use std::os::fd::{BorrowedFd, RawFd};
use btstack_gatt::Error;
use btstack_nusb::NusbHciTransport;

/// # Safety
/// raw_fd must be a valid, open USB FD, and its Android owner must not close it
/// concurrently with this call. Serialize access to the UsbDeviceConnection.
unsafe fn transport_from_android(raw_fd: RawFd) -> Result<NusbHciTransport, Error> {
    if raw_fd < 0 {
        return Err("UsbDeviceConnection is closed".into());
    }
    // SAFETY: guaranteed by the native integration's caller.
    let borrowed = unsafe { BorrowedFd::borrow_raw(raw_fd) };
    NusbHciTransport::from_borrowed_fd(borrowed)
}
```

The integer check alone does not prove the FD is valid: the JNI layer must
enforce the lifetime contract. The returned transport owns an independent FD
reference, not the Java-owned descriptor. The original and duplicate still refer
to the same USB connection; do not perform concurrent transfers, resets,
configuration changes, or interface operations through the Java connection.

Keep the `UsbDeviceConnection` open and strongly referenced until
`server.shutdown()` finishes. Then close it in the Android layer. Also retain
the Rust `GattServer`, consume its events, and shut it down on USB detach or when
the owning application component stops. Dropping the server also stops its thread.

Interface 0 is claimed using `claim_interface` on Android. A kernel driver already
holding the interface or device policy restrictions can still cause an error;
these conditions are returned to the caller rather than bypassed.

## Cross-build

Install Android NDK, the Rust Android targets, and `cargo-ndk`. Set `ANDROID_HOME`
to your SDK directory if it is not already discoverable.

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
cargo install cargo-ndk
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 --platform 23 build --workspace
```

For an integrating JNI crate, use `crate-type = ["cdylib"]` and build that crate
with `cargo ndk -o <app/src/main/jniLibs> ... build --release`. The library crates
in this workspace are ordinary Rust libraries to link into that native layer.

Verified on 2026-09-06: all four ABI workspace builds pass on a Windows host with
NDK `27.0.12077973`, Rust 1.95.0, and API level 23. Windows VID/PID startup also
passes against the physical `0411:0374` dongle. No Android device was connected;
USB permission, FD initialization, and GATT operation on Android hardware remain
unverified.
