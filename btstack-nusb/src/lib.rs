//! Native USB HCI transport.

use btstack_core::{Error, HciPacket, HciTransport};
use nusb::{
    MaybeFuture,
    transfer::{Bulk, ControlOut, ControlType, In, Interrupt, Out, Recipient},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug)]
pub struct UsbDeviceSelector {
    pub vendor_id: u16,
    pub product_id: u16,
}
impl UsbDeviceSelector {
    pub fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id,
            product_id,
        }
    }
}

pub struct NusbHciTransport {
    interface: nusb::Interface,
    acl_out: nusb::Endpoint<Bulk, Out>,
    incoming: mpsc::Receiver<Result<HciPacket, Error>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl NusbHciTransport {
    /// Opens a desktop USB controller selected by VID/PID.
    ///
    /// Android applications must obtain USB permission through UsbManager and
    /// use `from_fd` or `from_borrowed_fd` instead of enumeration.
    /// Multiple matching devices are rejected; use [`Self::from_device`] to pass
    /// a particular device selected by the application.
    #[cfg(not(target_os = "android"))]
    pub fn open(selector: UsbDeviceSelector) -> Result<Self, Error> {
        let mut matches = nusb::list_devices().wait()?.filter(|d| {
            d.vendor_id() == selector.vendor_id && d.product_id() == selector.product_id
        });
        let info = matches.next().ok_or_else(|| {
            format!(
                "USB Bluetooth controller {:04x}:{:04x} not found",
                selector.vendor_id, selector.product_id
            )
        })?;
        if matches.next().is_some() {
            return Err(format!("Multiple USB devices match {:04x}:{:04x}; select one explicitly and use NusbHciTransport::from_device", selector.vendor_id, selector.product_id).into());
        }
        let device = info.open().wait()?;
        Self::from_device(device)
    }

    /// Desktop enumeration is unavailable on Android. Use the permission-granted
    /// USB file descriptor through `from_fd` or `from_borrowed_fd`.
    #[cfg(target_os = "android")]
    pub fn open(_selector: UsbDeviceSelector) -> Result<Self, Error> {
        Err("Android requires a UsbManager file descriptor; use NusbHciTransport::from_fd or from_borrowed_fd".into())
    }

    /// Takes ownership of an open usbdevfs file descriptor on Android/Linux.
    ///
    /// Uses `nusb::Device::from_fd`; no enumeration or reopening by path occurs.
    /// The FD is released on failure or when the transport is dropped.
    /// Pass an owned duplicate, not a descriptor still owned by Java.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub fn from_fd(fd: std::os::fd::OwnedFd) -> Result<Self, Error> {
        Self::from_device(nusb::Device::from_fd(fd).wait()?)
    }

    /// Duplicates a borrowed usbdevfs FD before passing ownership to nusb.
    ///
    /// Useful with Android's `UsbDeviceConnection.getFileDescriptor()`. The
    /// caller must keep the original FD valid during this call. The transport
    /// owns only the duplicate and never closes the caller's FD. Do not perform
    /// competing USB operations through the original connection; keep that
    /// connection open until the GATT server has shut down.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub fn from_borrowed_fd(fd: std::os::fd::BorrowedFd<'_>) -> Result<Self, Error> {
        Self::from_fd(fd.try_clone_to_owned()?)
    }

    /// Creates an HCI transport from an already opened nusb device.
    ///
    /// Claims interface 0 and discovers its event and ACL endpoints. On Linux,
    /// detaches the kernel driver; Android uses the permission-granted FD and
    /// claims the interface without requesting a kernel-driver detach.
    pub fn from_device(device: nusb::Device) -> Result<Self, Error> {
        let descriptor = device.device_descriptor();
        let vendor_id = descriptor.vendor_id();
        let product_id = descriptor.product_id();
        #[cfg(target_os = "linux")]
        let interface = device.detach_and_claim_interface(0).wait()?;
        #[cfg(not(target_os = "linux"))]
        let interface = device.claim_interface(0).wait()?;
        let descriptor = interface
            .descriptors()
            .find(|d| d.alternate_setting() == 0)
            .ok_or("Missing interface 0 alternate setting 0")?;
        let mut event = None;
        let mut input = None;
        let mut output = None;
        for ep in descriptor.endpoints() {
            let addr = ep.address();
            match (ep.transfer_type(), addr & 0x80 != 0) {
                (nusb::descriptors::TransferType::Interrupt, true) => event = Some(addr),
                (nusb::descriptors::TransferType::Bulk, true) => input = Some(addr),
                (nusb::descriptors::TransferType::Bulk, false) => output = Some(addr),
                _ => {}
            }
        }
        let event = event.ok_or("Missing interrupt IN endpoint")?;
        let input = input.ok_or("Missing bulk IN endpoint")?;
        let output = output.ok_or("Missing bulk OUT endpoint")?;
        println!(
            "USB {:04x}:{:04x}, interface 0, event IN {event:#04x}, ACL IN {input:#04x}, ACL OUT {output:#04x}",
            vendor_id, product_id
        );
        let mut events = interface.endpoint::<Interrupt, In>(event)?;
        let mut acl = interface.endpoint::<Bulk, In>(input)?;
        let acl_out = interface.endpoint::<Bulk, Out>(output)?;
        let (tx, incoming) = mpsc::sync_channel(256);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::Builder::new()
            .name("hci-usb-rx".into())
            .spawn(move || {
                // Android permits USB transfers but may deny usbdevfs mmap.
                // Skip the zero-copy allocation attempt (and SELinux audit noise).
                #[cfg(target_os = "android")]
                let (event_buffer, acl_buffer) = (
                    nusb::transfer::Buffer::new(512),
                    nusb::transfer::Buffer::new(2048),
                );
                #[cfg(not(target_os = "android"))]
                let (event_buffer, acl_buffer) = (events.allocate(512), acl.allocate(2048));
                events.submit(event_buffer);
                acl.submit(acl_buffer);
                while !stopped.load(Ordering::Relaxed) {
                    for (kind, result) in [
                        (4, events.wait_next_complete(Duration::from_millis(2))),
                        (2, acl.wait_next_complete(Duration::from_millis(2))),
                    ] {
                        if let Some(result) = result {
                            if let Err(e) = result.status {
                                let _ = tx.try_send(Err(e.into()));
                                return;
                            }
                            let packet = HciPacket {
                                kind,
                                data: result.buffer.as_ref().to_vec(),
                            };
                            if !packet.data.is_empty() && tx.try_send(Ok(packet)).is_err() {
                                return;
                            }
                            // Completion preserves requested_len, even for short
                            // packets. Reuse the buffer after copying its payload.
                            if kind == 4 {
                                events.submit(result.buffer);
                            } else {
                                acl.submit(result.buffer);
                            }
                        }
                    }
                }
                events.cancel_all();
                acl.cancel_all();
            })?;
        Ok(Self {
            interface,
            acl_out,
            incoming,
            stop,
            worker: Some(worker),
        })
    }
}

impl HciTransport for NusbHciTransport {
    fn send(&mut self, kind: u8, packet: &[u8]) -> Result<(), Error> {
        match kind {
            1 => self
                .interface
                .control_out(
                    ControlOut {
                        control_type: ControlType::Class,
                        recipient: Recipient::Device,
                        request: 0,
                        value: 0,
                        index: 0,
                        data: packet,
                    },
                    Duration::from_secs(2),
                )
                .wait()?,
            2 => {
                self.acl_out.submit(packet.to_vec().into());
                let result = self
                    .acl_out
                    .wait_next_complete(Duration::from_secs(2))
                    .ok_or("ACL OUT timeout")?;
                result.status?;
                if result.actual_len != packet.len() {
                    return Err("Short ACL OUT transfer".into());
                }
            }
            _ => return Err("Unsupported HCI packet type".into()),
        }
        Ok(())
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<HciPacket>, Error> {
        match self.incoming.recv_timeout(timeout) {
            Ok(packet) => packet.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("USB receive worker stopped (device removed or queue overflow)".into()),
        }
    }
}

impl Drop for NusbHciTransport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
