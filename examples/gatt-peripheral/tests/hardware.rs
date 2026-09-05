use btstack_gatt::{Error, GattCharacteristic, GattServer, GattService};
use btstack_nusb::{NusbHciTransport, UsbDeviceSelector};

#[test]
#[ignore = "requires exclusive access to USB 0411:0374"]
fn start_stop_twice_in_one_process() -> Result<(), Error> {
    for _ in 0..2 {
        let usb = NusbHciTransport::open(UsbDeviceSelector::new(0x0411, 0x0374))?;
        let mut server = GattServer::builder(usb)
            .service(
                GattService::new([1; 16]).characteristic(
                    GattCharacteristic::new([2; 16])
                        .read()
                        .on_read(|_| Ok(b"test".to_vec())),
                ),
            )
            .start()?;
        server.shutdown()?;
    }
    Ok(())
}
