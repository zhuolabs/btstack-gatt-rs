//! Single-threaded BTstack runtime and transport boundary.

mod advertising;
mod gatt;
pub use advertising::AdvertisingData;
mod runtime;
pub use gatt::*;
pub use runtime::Runtime;

use std::time::Duration;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct HciPacket {
    pub kind: u8,
    pub data: Vec<u8>,
}

/// Sends complete HCI packets synchronously; receives owned packets from USB workers.
pub trait HciTransport: Send + 'static {
    fn send(&mut self, kind: u8, packet: &[u8]) -> Result<(), Error>;
    fn receive(&mut self, timeout: Duration) -> Result<Option<HciPacket>, Error>;
}
