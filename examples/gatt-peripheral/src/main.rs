fn main() {
    let _usb =
        btstack_nusb::NusbHciTransport::open(btstack_nusb::UsbDeviceSelector::new(0x0411, 0x0374))
            .unwrap();
}
