use crate::{gatt::validate, *};
use btstack_sys as sys;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static TRANSPORT: RefCell<Option<Box<dyn HciTransport>>> = RefCell::new(None);
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static EPOCH: Instant = Instant::now();
    static EVENTS: RefCell<VecDeque<(u8,u16,u16)>> = const { RefCell::new(VecDeque::new()) };
    static FAILURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct Attribute {
    handle: u16,
    characteristic: GattCharacteristic,
}
struct State {
    attributes: Vec<Attribute>,
    connections: HashMap<u16, GattConnection>,
    subscriptions: HashMap<(u16, u16), bool>,
    reads: HashMap<(u16, u16), Vec<u8>>,
    pending: VecDeque<(GattConnection, u16, Vec<u8>)>,
    events: mpsc::Sender<ServerEvent>,
}
enum Command {
    Stop,
    Notify(
        GattConnection,
        Uuid,
        Vec<u8>,
        mpsc::SyncSender<Result<(), String>>,
    ),
}

pub struct Runtime {
    commands: mpsc::SyncSender<Command>,
    events: mpsc::Receiver<ServerEvent>,
    worker: Option<thread::JoinHandle<Result<(), Error>>>,
}

impl Runtime {
    /// Returns only after LE Set Advertising Enable completes successfully.
    pub fn start(
        transport: impl HciTransport,
        name: String,
        services: Vec<GattService>,
    ) -> Result<Self, Error> {
        let adv = validate(&name, &services)?;
        if ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Only one BTstack runtime may be active per process".into());
        }
        let (commands, rx) = mpsc::sync_channel(128);
        let (event_tx, events) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("btstack".into())
            .spawn(move || {
                struct ActiveGuard;
                impl Drop for ActiveGuard {
                    fn drop(&mut self) {
                        ACTIVE.store(false, Ordering::Release);
                    }
                }
                let _active = ActiveGuard;
                TRANSPORT.with(|t| *t.borrow_mut() = Some(Box::new(transport)));
                STATE.with(|s| {
                    *s.borrow_mut() = Some(State {
                        attributes: Vec::new(),
                        connections: HashMap::new(),
                        subscriptions: HashMap::new(),
                        reads: HashMap::new(),
                        pending: VecDeque::new(),
                        events: event_tx.clone(),
                    })
                });
                // SAFETY: this thread exclusively owns BTstack for the entire lifecycle.
                unsafe {
                    sys::rs_init(send, time_ms, event);
                    sys::rs_gap_name(name.as_ptr(), name.len() as u16);
                    for service in services {
                        sys::rs_service(service.uuid.as_ptr());
                        for characteristic in service.characteristics {
                            let handle = sys::rs_characteristic(
                                characteristic.uuid.as_ptr(),
                                characteristic.properties,
                            );
                            STATE.with(|s| {
                                s.borrow_mut().as_mut().unwrap().attributes.push(Attribute {
                                    handle,
                                    characteristic,
                                })
                            });
                        }
                    }
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    if unsafe { sys::rs_start(read, write, adv.as_ptr(), adv.len() as u8) } != 0 {
                        return Err("HCI power-on failed".into());
                    }
                    run(rx, ready_tx)
                }))
                .unwrap_or_else(|_| Err("BTstack runtime callback panicked".into()));
                let shutdown_result = catch_unwind(AssertUnwindSafe(stop_controller))
                    .unwrap_or_else(|_| Err("Transport panicked during shutdown".into()));
                let result = result.and(shutdown_result);
                if let Err(e) = &result {
                    let _ = event_tx.send(ServerEvent::Error(format!("{e}")));
                }
                unsafe {
                    sys::rs_deinit();
                }
                STATE.with(|s| *s.borrow_mut() = None);
                TRANSPORT.with(|t| *t.borrow_mut() = None);
                result
            });
        let worker = match worker {
            Ok(w) => w,
            Err(e) => {
                ACTIVE.store(false, Ordering::Release);
                return Err(e.into());
            }
        };
        let mut runtime = Self {
            commands,
            events,
            worker: Some(worker),
        };
        match ready.recv_timeout(Duration::from_secs(25)) {
            Ok(()) => Ok(runtime),
            Err(_) => {
                let result = runtime.shutdown();
                Err(result
                    .err()
                    .unwrap_or_else(|| "Controller advertising startup timed out".into()))
            }
        }
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ServerEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
    /// Success means enqueued, not transmitted. Requires a live subscribed peer.
    pub fn notify(
        &self,
        connection: &GattConnection,
        characteristic: Uuid,
        value: &[u8],
    ) -> Result<(), Error> {
        if STATE.with(|s| s.borrow().is_some()) {
            return Err("Do not call blocking server methods from a GATT callback".into());
        }
        if value.len() > 512 {
            return Err("ATT value exceeds 512 bytes".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.commands
            .try_send(Command::Notify(
                connection.clone(),
                characteristic,
                value.to_vec(),
                tx,
            ))
            .map_err(|_| "Runtime stopped or command queue full")?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "Notification enqueue timed out")?
            .map_err(Into::into)
    }
    pub fn shutdown(&mut self) -> Result<(), Error> {
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.thread().id() == thread::current().id())
        {
            let _ = self.commands.try_send(Command::Stop);
            return Err("Cannot join the BTstack thread from its own callback".into());
        }
        if let Some(worker) = self.worker.take() {
            let _ = self.commands.send(Command::Stop);
            worker.join().map_err(|_| "BTstack thread panicked")??;
        }
        Ok(())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn stop_controller() -> Result<(), Error> {
    unsafe {
        sys::rs_stop();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while unsafe { sys::rs_is_off() } == 0 {
        if Instant::now() >= deadline {
            return Err("Controller shutdown timed out".into());
        }
        let packet = TRANSPORT.with(|t| {
            t.borrow_mut()
                .as_mut()
                .unwrap()
                .receive(Duration::from_millis(5))
        })?;
        if let Some(mut packet) = packet {
            validate_packet(&packet)?;
            if std::env::var_os("BTSTACK_HCI_TRACE").is_some() {
                eprintln!("HCI RX {} {:02x?}", packet.kind, packet.data);
            }
            unsafe {
                sys::rs_receive(
                    packet.kind,
                    packet.data.as_mut_ptr(),
                    packet.data.len() as u16,
                );
            }
        }
        unsafe {
            sys::rs_poll();
        }
        if let Some(error) = FAILURE.with(|f| f.borrow_mut().take()) {
            return Err(error.into());
        }
    }
    println!("BTstack HCI_STATE_OFF (controller shutdown complete)");
    Ok(())
}

fn run(commands: mpsc::Receiver<Command>, ready: mpsc::SyncSender<()>) -> Result<(), Error> {
    let start = Instant::now();
    let mut started = false;
    loop {
        for _ in 0..128 {
            let command = match commands.try_recv() {
                Ok(command) => command,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            };
            match command {
                Command::Stop => return Ok(()),
                Command::Notify(connection, uuid, value, response) => {
                    let result = STATE.with(|s| {
                        let mut state = s.borrow_mut();
                        let s = state.as_mut().unwrap();
                        let current = s
                            .connections
                            .get(&connection.handle)
                            .filter(|c| c.generation == connection.generation)
                            .ok_or("Connection is no longer active")?;
                        let attr = s
                            .attributes
                            .iter()
                            .find(|a| a.characteristic.uuid == uuid)
                            .ok_or("Unknown characteristic")?;
                        if !s
                            .subscriptions
                            .get(&(connection.handle, attr.handle))
                            .copied()
                            .unwrap_or(false)
                        {
                            return Err("Peer has not subscribed");
                        }
                        if value.len() > current.mtu.saturating_sub(3) as usize {
                            return Err("Notification exceeds negotiated ATT MTU");
                        }
                        if s.pending.len() >= 128 {
                            return Err("Notification queue full");
                        }
                        s.pending.push_back((current.clone(), attr.handle, value));
                        Ok(())
                    });
                    if result.is_ok() {
                        unsafe {
                            sys::rs_request_send(connection.handle);
                        }
                    }
                    let _ = response.send(result.map_err(str::to_owned));
                }
            }
        }
        let packet = TRANSPORT.with(|t| {
            t.borrow_mut()
                .as_mut()
                .unwrap()
                .receive(Duration::from_millis(5))
        })?;
        if let Some(mut packet) = packet {
            validate_packet(&packet)?;
            if std::env::var_os("BTSTACK_HCI_TRACE").is_some() {
                eprintln!("HCI RX {} {:02x?}", packet.kind, packet.data);
            }
            unsafe {
                sys::rs_receive(
                    packet.kind,
                    packet.data.as_mut_ptr(),
                    packet.data.len() as u16,
                );
            }
        }
        unsafe {
            sys::rs_poll();
        }
        if let Some(error) = FAILURE.with(|f| f.borrow_mut().take()) {
            return Err(error.into());
        }
        while let Some((kind, handle, value)) = EVENTS.with(|e| e.borrow_mut().pop_front()) {
            match kind {
                1 => println!("BTstack HCI_STATE_WORKING"),
                2 => {
                    if value != 0 {
                        return Err(
                            format!("LE advertising failed: HCI status {value:#04x}").into()
                        );
                    }
                    if !started {
                        started = true;
                        println!(
                            "GATT server ready: LE advertising enabled (controller status 0x00)"
                        );
                        let _ = ready.send(());
                    }
                }
                3 | 4 | 7 => STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    let s = state.as_mut().unwrap();
                    if kind == 3 {
                        let c = GattConnection {
                            handle,
                            generation: NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed),
                            mtu: value,
                        };
                        s.connections.insert(handle, c.clone());
                        let _ = s.events.send(ServerEvent::Connected(c));
                    } else if kind == 4 {
                        s.subscriptions.retain(|(c, _), _| *c != handle);
                        s.reads.retain(|(c, _), _| *c != handle);
                        s.pending.retain(|(c, _, _)| c.handle != handle);
                        if let Some(c) = s.connections.remove(&handle) {
                            let _ = s.events.send(ServerEvent::Disconnected(c));
                        }
                    } else if let Some(c) = s.connections.get_mut(&handle) {
                        c.mtu = value;
                    }
                }),
                5 => flush_notification(handle),
                6 => return Err(format!("HCI command {handle:#06x} failed: {value:#04x}").into()),
                _ => {}
            }
        }
        if !started && start.elapsed() > Duration::from_secs(20) {
            return Err("Timed out initializing controller / advertising".into());
        }
    }
}

fn validate_packet(packet: &HciPacket) -> Result<(), Error> {
    let expected = match packet.kind {
        4 if packet.data.len() >= 2 => 2 + packet.data[1] as usize,
        2 if packet.data.len() >= 4 => {
            4 + u16::from_le_bytes([packet.data[2], packet.data[3]]) as usize
        }
        _ => return Err("Invalid HCI packet header".into()),
    };
    if packet.data.len() != expected || (packet.kind == 2 && expected > 1028) {
        return Err("Invalid HCI packet length".into());
    }
    Ok(())
}

fn flush_notification(handle: u16) {
    let next = STATE.with(|s| {
        let mut state = s.borrow_mut();
        let s = state.as_mut().unwrap();
        let index = s.pending.iter().position(|(c, _, _)| c.handle == handle)?;
        s.pending.remove(index)
    });
    if let Some((connection, attribute, value)) = next {
        let subscribed = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .unwrap()
                .subscriptions
                .get(&(handle, attribute))
                .copied()
                .unwrap_or(false)
        });
        if subscribed {
            let status =
                unsafe { sys::rs_notify(handle, attribute, value.as_ptr(), value.len() as u16) };
            if status != 0 {
                STATE.with(|s| {
                    let _ = s
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .events
                        .send(ServerEvent::NotificationFailed { connection, status });
                });
            }
        }
        let more = STATE.with(|s| {
            s.borrow()
                .as_ref()
                .unwrap()
                .pending
                .iter()
                .any(|(c, _, _)| c.handle == handle)
        });
        if more {
            unsafe {
                sys::rs_request_send(handle);
            }
        }
    }
}

unsafe extern "C" fn time_ms() -> u32 {
    EPOCH.with(|e| e.elapsed().as_millis() as u32)
}
unsafe extern "C" fn event(kind: u8, handle: u16, value: u16) {
    EVENTS.with(|e| e.borrow_mut().push_back((kind, handle, value)));
}
unsafe extern "C" fn send(kind: u8, data: *const u8, len: u16) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let packet = unsafe { std::slice::from_raw_parts(data, len as usize) };
        if std::env::var_os("BTSTACK_HCI_TRACE").is_some() {
            eprintln!("HCI TX {kind} {packet:02x?}");
        }
        TRANSPORT.with(|t| t.borrow_mut().as_mut().unwrap().send(kind, packet))
    }));
    match result {
        Ok(Ok(())) => 0,
        other => {
            let message = match other {
                Ok(Err(e)) => e.to_string(),
                _ => "Transport callback panicked".into(),
            };
            FAILURE.with(|f| *f.borrow_mut() = Some(message));
            -1
        }
    }
}

unsafe extern "C" fn read(
    connection: u16,
    attribute: u16,
    offset: u16,
    buffer: *mut u8,
    size: u16,
) -> u16 {
    catch_unwind(AssertUnwindSafe(|| {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let s = state.as_mut().unwrap();
            let Some(c) = s.connections.get(&connection) else {
                return 0xfe0e;
            };
            let Some(a) = s.attributes.iter().find(|a| {
                a.handle == attribute
                    || (a.characteristic.properties & 16 != 0 && a.handle + 1 == attribute)
            }) else {
                return 0xfe02;
            };
            let key = (connection, attribute);
            if buffer.is_null() || !s.reads.contains_key(&key) {
                let value = if a.handle != attribute {
                    vec![
                        u8::from(
                            s.subscriptions
                                .get(&(connection, a.handle))
                                .copied()
                                .unwrap_or(false),
                        ),
                        0,
                    ]
                } else {
                    match a
                        .characteristic
                        .read
                        .as_ref()
                        .ok_or(GattStatus::ReadNotPermitted)
                        .and_then(|f| f(GattReadRequest { connection: c }))
                    {
                        Ok(value) if value.len() <= 512 => value,
                        Ok(_) => return 0xfe0d,
                        Err(e) => return 0xfe00 + e as u16,
                    }
                };
                s.reads.insert(key, value);
            }
            let value = &s.reads[&key];
            if offset as usize > value.len() {
                return 0xfe07;
            }
            if buffer.is_null() {
                return value.len() as u16;
            }
            let value = &value[offset as usize..];
            let len = value.len().min(size as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), buffer, len);
            }
            len as u16
        })
    }))
    .unwrap_or(0xfe0e)
}

unsafe extern "C" fn write(
    connection: u16,
    attribute: u16,
    mode: u16,
    offset: u16,
    buffer: *mut u8,
    size: u16,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if mode != 0 {
            return GattStatus::RequestNotSupported as i32;
        }
        if offset != 0 {
            return GattStatus::InvalidOffset as i32;
        }
        let value = if size == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(buffer, size as usize) }
        };
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let s = state.as_mut().unwrap();
            let Some(c) = s.connections.get(&connection) else {
                return GattStatus::UnlikelyError as i32;
            };
            let Some(a) = s.attributes.iter().find(|a| {
                a.handle == attribute
                    || (a.characteristic.properties & 16 != 0 && a.handle + 1 == attribute)
            }) else {
                return GattStatus::WriteNotPermitted as i32;
            };
            if a.handle != attribute {
                if value.len() != 2 {
                    return GattStatus::InvalidValueLength as i32;
                }
                if value[1] != 0 || value[0] > 1 {
                    return GattStatus::ValueNotAllowed as i32;
                }
                let enabled = value[0] == 1;
                s.subscriptions.insert((connection, a.handle), enabled);
                s.reads.remove(&(connection, attribute));
                let _ = s.events.send(ServerEvent::SubscriptionChanged {
                    connection: c.clone(),
                    characteristic: a.characteristic.uuid,
                    subscription: if enabled {
                        SubscriptionType::Notify
                    } else {
                        SubscriptionType::None
                    },
                });
                return 0;
            }
            if value.len() > 512 {
                return GattStatus::InvalidValueLength as i32;
            }
            match a
                .characteristic
                .write
                .as_ref()
                .ok_or(GattStatus::WriteNotPermitted)
                .and_then(|f| {
                    f(GattWriteRequest {
                        connection: c,
                        value,
                    })
                }) {
                Ok(()) => {
                    s.reads.remove(&(connection, attribute));
                    0
                }
                Err(e) => e as i32,
            }
        })
    }))
    .unwrap_or(GattStatus::UnlikelyError as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn setup(characteristic: GattCharacteristic) -> mpsc::Receiver<ServerEvent> {
        let (tx, rx) = mpsc::channel();
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                attributes: vec![Attribute {
                    handle: 10,
                    characteristic,
                }],
                connections: [1, 2]
                    .into_iter()
                    .map(|handle| {
                        (
                            handle,
                            GattConnection {
                                handle,
                                generation: handle as u64,
                                mtu: 23,
                            },
                        )
                    })
                    .collect(),
                subscriptions: HashMap::new(),
                reads: HashMap::new(),
                pending: VecDeque::new(),
                events: tx,
            })
        });
        rx
    }

    #[test]
    fn read_probe_and_copy_use_one_snapshot_and_handle_offsets() {
        let calls = Arc::new(AtomicU64::new(0));
        let count = calls.clone();
        let _events = setup(GattCharacteristic::new([1; 16]).read().on_read(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(b"abcdef".to_vec())
        }));
        unsafe {
            assert_eq!(read(1, 10, 0, std::ptr::null_mut(), 0), 6);
            let mut buffer = [0; 3];
            assert_eq!(read(1, 10, 2, buffer.as_mut_ptr(), 3), 3);
            assert_eq!(&buffer, b"cde");
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(read(1, 10, 7, buffer.as_mut_ptr(), 3), 0xfe07);
            assert_eq!(read(1, 10, 6, buffer.as_mut_ptr(), 3), 0);
        }
    }

    #[test]
    fn cccd_is_per_connection_and_rejects_unsupported_bits() {
        let events = setup(GattCharacteristic::new([1; 16]).notify());
        unsafe {
            assert_eq!(write(1, 11, 0, 0, [1, 0].as_mut_ptr(), 2), 0);
            assert!(matches!(
                events.try_recv().unwrap(),
                ServerEvent::SubscriptionChanged {
                    subscription: SubscriptionType::Notify,
                    ..
                }
            ));
            let mut buffer = [0; 2];
            assert_eq!(read(1, 11, 0, buffer.as_mut_ptr(), 2), 2);
            assert_eq!(buffer, [1, 0]);
            assert_eq!(read(2, 11, 0, buffer.as_mut_ptr(), 2), 2);
            assert_eq!(buffer, [0, 0]);
            assert_eq!(write(1, 11, 0, 0, [2, 0].as_mut_ptr(), 2), 19);
            assert_eq!(write(1, 11, 0, 0, [1].as_mut_ptr(), 1), 13);
            assert_eq!(write(1, 11, 0, 0, [0, 0].as_mut_ptr(), 2), 0);
            assert_eq!(read(1, 11, 0, buffer.as_mut_ptr(), 2), 2);
            assert_eq!(buffer, [0, 0]);
        }
    }

    #[test]
    fn prepared_and_offset_writes_never_invoke_application() {
        let calls = Arc::new(AtomicU64::new(0));
        let count = calls.clone();
        let _events = setup(GattCharacteristic::new([1; 16]).write().on_write(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }));
        unsafe {
            assert_eq!(write(1, 10, 1, 0, [1].as_mut_ptr(), 1), 6);
            assert_eq!(write(1, 10, 0, 1, [1].as_mut_ptr(), 1), 7);
            assert_eq!(calls.load(Ordering::Relaxed), 0);
            assert_eq!(write(1, 10, 0, 0, std::ptr::null_mut(), 0), 0);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn application_panics_do_not_cross_c_boundary() {
        let _events = setup(
            GattCharacteristic::new([1; 16])
                .read()
                .write()
                .on_read(|_| panic!("read panic"))
                .on_write(|_| panic!("write panic")),
        );
        unsafe {
            assert_eq!(read(1, 10, 0, std::ptr::null_mut(), 0), 0xfe0e);
            assert_eq!(write(1, 10, 0, 0, std::ptr::null_mut(), 0), 14);
        }
    }

    #[test]
    fn reject_malformed_hci_lengths() {
        assert!(
            validate_packet(&HciPacket {
                kind: 4,
                data: vec![0x0e, 4, 1, 3, 0x0c, 0]
            })
            .is_ok()
        );
        for packet in [
            HciPacket {
                kind: 4,
                data: vec![0x0e],
            },
            HciPacket {
                kind: 4,
                data: vec![0x0e, 4, 0],
            },
            HciPacket {
                kind: 2,
                data: vec![0, 0, 10, 0, 1],
            },
            HciPacket {
                kind: 3,
                data: vec![0, 0],
            },
        ] {
            assert!(validate_packet(&packet).is_err());
        }
    }
}
