use btstack_gatt::{
    Error, GattCharacteristic, GattServer, GattService, ServerEvent, SubscriptionType,
};
use btstack_nusb::{NusbHciTransport, UsbDeviceSelector};
use std::time::{Duration, Instant};

// Nordic UART service UUIDs, in canonical big-endian order.
const SERVICE: [u8; 16] = [
    0x6e, 0x40, 0, 1, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];
const RX: [u8; 16] = [
    0x6e, 0x40, 0, 2, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];
const TX: [u8; 16] = [
    0x6e, 0x40, 0, 3, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];

fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().collect();
    let seconds = args
        .windows(2)
        .find(|a| a[0] == "--seconds")
        .map(|a| a[1].parse::<u64>())
        .transpose()?;
    let usb = NusbHciTransport::open(UsbDeviceSelector::new(0x0411, 0x0374))?;
    if args.iter().any(|a| a == "--probe") {
        return Ok(());
    }
    let mut server = GattServer::builder(usb)
        .name("Rust GATT")
        .service(
            GattService::new(SERVICE)
                .characteristic(
                    GattCharacteristic::new(RX)
                        .write()
                        .write_without_response()
                        .on_write(|req| {
                            println!(
                                "RX from connection {}: {:02x?}",
                                req.connection.id(),
                                req.value()
                            );
                            Ok(())
                        }),
                )
                .characteristic(
                    GattCharacteristic::new(TX)
                        .read()
                        .notify()
                        .on_read(|_| Ok(b"hello from Rust".to_vec())),
                ),
        )
        .start()?;
    println!(
        "Advertising as Rust GATT; RX write, TX read + notify. Use --seconds N for a bounded run."
    );
    let start = Instant::now();
    let mut tick = Instant::now();
    let mut subscribers = std::collections::HashMap::new();
    while seconds.is_none_or(|s| start.elapsed() < Duration::from_secs(s)) {
        match server.recv_timeout(Duration::from_millis(100)) {
            Ok(ServerEvent::SubscriptionChanged {
                connection,
                subscription,
                ..
            }) => {
                println!(
                    "Connection {} subscription: {subscription:?}",
                    connection.id()
                );
                if subscription == SubscriptionType::Notify {
                    subscribers.insert(connection.id(), connection);
                } else {
                    subscribers.remove(&connection.id());
                }
            }
            Ok(ServerEvent::Disconnected(connection)) => {
                subscribers.remove(&connection.id());
                println!("Disconnected {}", connection.id());
            }
            Ok(ServerEvent::Error(error)) => return Err(error.into()),
            Ok(event) => println!("{event:?}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("BTstack runtime stopped".into()),
        }
        if tick.elapsed() >= Duration::from_secs(1) {
            let value = format!("tick {}", start.elapsed().as_secs());
            for connection in subscribers.values() {
                if let Err(e) = server.notify(connection, TX, value.as_bytes()) {
                    eprintln!("Notify: {e}");
                }
            }
            tick = Instant::now();
        }
    }
    server.shutdown()?;
    println!("GATT server stopped; USB interface released");
    Ok(())
}
