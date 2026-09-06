use btstack_gatt::{
    Error, GattCharacteristic, GattServer, GattService, ServerEvent, SubscriptionType,
};
use btstack_nusb::NusbHciTransport;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    os::fd::{FromRawFd, OwnedFd},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const SERVICE: [u8; 16] = [
    0x6e, 0x40, 0, 1, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];
const RX: [u8; 16] = [
    0x6e, 0x40, 0, 2, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];
const TX: [u8; 16] = [
    0x6e, 0x40, 0, 3, 0xb5, 0xa3, 0xf3, 0x93, 0xe0, 0xa9, 0xe5, 0x0e, 0x24, 0xdc, 0xca, 0x9e,
];

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        priority: i32,
        tag: *const libc::c_char,
        text: *const libc::c_char,
    ) -> i32;
}

/// Android discards native stdout by default. Forward stdout/stderr, including
/// BTstack C output, to logcat for the lifetime of this sample's process.
pub fn redirect_output() -> Result<(), String> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        let mut pipe = [-1; 2];
        if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let reader = unsafe { std::fs::File::from_raw_fd(pipe[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
        std::thread::Builder::new()
            .name("rust-logcat".into())
            .spawn(move || {
                for line in BufReader::new(reader).split(b'\n').flatten() {
                    let line: Vec<_> = line.into_iter().filter(|b| *b != 0).collect();
                    if let Ok(message) = std::ffi::CString::new(line) {
                        unsafe {
                            __android_log_write(4, c"BtstackGatt".as_ptr(), message.as_ptr());
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        use std::os::fd::AsRawFd;
        for destination in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            if unsafe { libc::dup2(writer.as_raw_fd(), destination) } < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    })
    .clone()
}

pub fn serve(fd: OwnedFd, stop: &AtomicBool) -> Result<(), Error> {
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let usb = NusbHciTransport::from_fd(fd)?;
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
                                "on_write connection {}: {:02x?}",
                                req.connection.id(),
                                req.value()
                            );
                            Ok(())
                        }),
                )
                .characteristic(GattCharacteristic::new(TX).read().notify().on_read(|_| {
                    println!("on_read TX");
                    Ok(b"hello from Rust".to_vec())
                })),
        )
        .start()?;
    println!("Advertising as Rust GATT; RX write, TX read + notify");
    let start = Instant::now();
    let mut tick = Instant::now();
    let mut subscribers = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        match server.recv_timeout(Duration::from_millis(100)) {
            Ok(ServerEvent::SubscriptionChanged {
                connection,
                subscription,
                ..
            }) => {
                println!(
                    "SubscriptionChanged connection {}: {subscription:?}",
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
                if let Err(error) = server.notify(connection, TX, value.as_bytes()) {
                    eprintln!("Notify: {error}");
                }
            }
            tick = Instant::now();
        }
    }
    server.shutdown()?;
    println!("GATT server stopped; USB interface released");
    Ok(())
}
