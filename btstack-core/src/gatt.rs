use crate::Error;

/// Canonical, big-endian 128-bit Bluetooth UUID.
pub type Uuid = [u8; 16];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GattConnection {
    pub(crate) handle: u16,
    pub(crate) generation: u64,
    pub(crate) mtu: u16,
}
impl GattConnection {
    pub fn id(&self) -> u64 {
        self.generation
    }
    pub fn mtu(&self) -> u16 {
        self.mtu
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionType {
    None,
    Notify,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum GattStatus {
    ReadNotPermitted = 2,
    WriteNotPermitted = 3,
    InvalidOffset = 7,
    RequestNotSupported = 6,
    InvalidValueLength = 13,
    UnlikelyError = 14,
    ValueNotAllowed = 19,
}

pub struct GattReadRequest<'a> {
    pub connection: &'a GattConnection,
}
/// Only ordinary writes at offset zero are supported initially. BTstack does
/// not distinguish write request vs command in this callback API.
pub struct GattWriteRequest<'a> {
    pub connection: &'a GattConnection,
    pub value: &'a [u8],
}
impl GattWriteRequest<'_> {
    pub fn value(&self) -> &[u8] {
        self.value
    }
}

pub type ReadHandler = Box<dyn Fn(GattReadRequest<'_>) -> Result<Vec<u8>, GattStatus> + Send>;
pub type WriteHandler = Box<dyn Fn(GattWriteRequest<'_>) -> Result<(), GattStatus> + Send>;

pub struct GattCharacteristic {
    pub(crate) uuid: Uuid,
    pub(crate) properties: u16,
    pub(crate) read: Option<ReadHandler>,
    pub(crate) write: Option<WriteHandler>,
}
impl GattCharacteristic {
    pub fn new(uuid: Uuid) -> Self {
        Self {
            uuid,
            properties: 0,
            read: None,
            write: None,
        }
    }
    pub fn read(mut self) -> Self {
        self.properties |= 2;
        self
    }
    pub fn write(mut self) -> Self {
        self.properties |= 8;
        self
    }
    pub fn write_without_response(mut self) -> Self {
        self.properties |= 4;
        self
    }
    pub fn notify(mut self) -> Self {
        self.properties |= 16;
        self
    }
    /// Return the entire value. The wrapper handles ATT length probes and offsets.
    pub fn on_read(
        mut self,
        handler: impl Fn(GattReadRequest<'_>) -> Result<Vec<u8>, GattStatus> + Send + 'static,
    ) -> Self {
        self.read = Some(Box::new(handler));
        self
    }
    pub fn on_write(
        mut self,
        handler: impl Fn(GattWriteRequest<'_>) -> Result<(), GattStatus> + Send + 'static,
    ) -> Self {
        self.write = Some(Box::new(handler));
        self
    }
}

pub struct GattService {
    pub(crate) uuid: Uuid,
    pub(crate) characteristics: Vec<GattCharacteristic>,
}
impl GattService {
    pub fn new(uuid: Uuid) -> Self {
        Self {
            uuid,
            characteristics: Vec::new(),
        }
    }
    pub fn characteristic(mut self, characteristic: GattCharacteristic) -> Self {
        self.characteristics.push(characteristic);
        self
    }
}

#[derive(Debug)]
pub enum ServerEvent {
    Connected(GattConnection),
    Disconnected(GattConnection),
    SubscriptionChanged {
        connection: GattConnection,
        characteristic: Uuid,
        subscription: SubscriptionType,
    },
    NotificationFailed {
        connection: GattConnection,
        status: u8,
    },
    Error(String),
}

pub(crate) fn validate(name: &str, services: &[GattService]) -> Result<Vec<u8>, Error> {
    if name.is_empty() || name.len() > 26 {
        return Err("Advertising name must contain 1..=26 UTF-8 bytes".into());
    }
    if services.is_empty() {
        return Err("At least one GATT service is required".into());
    }
    let mut uuids = std::collections::HashSet::new();
    let mut count = 0;
    for service in services {
        if service.characteristics.is_empty() {
            return Err("Service requires a characteristic".into());
        }
        for c in &service.characteristics {
            count += 1;
            if !uuids.insert(c.uuid) {
                return Err("Characteristic UUIDs must be unique".into());
            }
            if c.properties == 0
                || (c.properties & 2 != 0 && c.read.is_none())
                || (c.properties & 12 != 0 && c.write.is_none())
            {
                return Err("Characteristic properties require corresponding handlers".into());
            }
        }
    }
    if count > 64 || services.len() > 16 {
        return Err(
            "Initial implementation supports at most 16 services and 64 characteristics".into(),
        );
    }
    let mut adv = vec![2, 1, 6, (name.len() + 1) as u8, 9];
    adv.extend_from_slice(name.as_bytes());
    Ok(adv)
}
