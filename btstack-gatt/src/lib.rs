//! Safe GATT peripheral API.

pub use btstack_core::{
    Error, GattCharacteristic, GattConnection, GattReadRequest, GattService, GattStatus,
    GattWriteRequest, HciTransport, ServerEvent, SubscriptionType, Uuid,
};

pub struct GattServer {
    runtime: btstack_core::Runtime,
}
pub struct GattServerBuilder<T> {
    transport: T,
    name: String,
    services: Vec<GattService>,
}
impl GattServer {
    pub fn builder<T: HciTransport>(transport: T) -> GattServerBuilder<T> {
        GattServerBuilder {
            transport,
            name: "Rust GATT".into(),
            services: Vec::new(),
        }
    }
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<ServerEvent, std::sync::mpsc::RecvTimeoutError> {
        self.runtime.recv_timeout(timeout)
    }
    pub fn notify(
        &self,
        connection: &GattConnection,
        characteristic: Uuid,
        value: &[u8],
    ) -> Result<(), Error> {
        self.runtime.notify(connection, characteristic, value)
    }
    pub fn shutdown(&mut self) -> Result<(), Error> {
        self.runtime.shutdown()
    }
}
impl<T: HciTransport> GattServerBuilder<T> {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    pub fn service(mut self, service: GattService) -> Self {
        self.services.push(service);
        self
    }
    pub fn start(self) -> Result<GattServer, Error> {
        Ok(GattServer {
            runtime: btstack_core::Runtime::start(self.transport, self.name, self.services)?,
        })
    }
}
