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
    pub fn open(selector: UsbDeviceSelector) -> Result<Self, Error> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| d.vendor_id() == selector.vendor_id && d.product_id() == selector.product_id)
            .ok_or("USB Bluetooth controller not found")?;
        let device = info.open().wait()?;
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
            selector.vendor_id, selector.product_id
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
                events.submit(events.allocate(512));
                acl.submit(acl.allocate(2048));
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
                            if kind == 4 {
                                events.submit(events.allocate(512));
                            } else {
                                acl.submit(acl.allocate(2048));
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
