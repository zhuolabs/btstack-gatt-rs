use btstack_gatt::{
    Error, GattCharacteristic, GattServer, GattService, ServerEvent, SubscriptionType,
};
use btstack_nusb::NusbHciTransport;
use std::time::{Duration, Instant};

mod args;

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
    let args = args::Args::parse(std::env::args().skip(1))?;
    if args.help {
        println!("{}", args::USAGE);
        return Ok(());
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_signal = stop.clone();
    ctrlc::set_handler(move || stop_signal.store(true, std::sync::atomic::Ordering::Relaxed))?;
    let usb = NusbHciTransport::open(args.selector)?;
    if args.probe {
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
    while !stop.load(std::sync::atomic::Ordering::Relaxed)
        && args
            .seconds
            .is_none_or(|s| start.elapsed() < Duration::from_secs(s))
    {
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
