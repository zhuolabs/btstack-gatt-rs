//! Safe GATT peripheral API.

pub use btstack_core::{
    AdvertisingData, Error, GattCharacteristic, GattConnection, GattReadRequest, GattService,
    GattStatus, GattWriteRequest, HciTransport, ServerEvent, SubscriptionType, Uuid,
};

pub struct GattServer {
    runtime: btstack_core::Runtime,
}

pub struct GattServerBuilder<T> {
    transport: T,
    name: String,
    services: Vec<GattService>,
    advertising: Option<AdvertisingData>,
}
impl GattServer {
    pub fn builder<T: HciTransport>(transport: T) -> GattServerBuilder<T> {
        GattServerBuilder {
            transport,
            name: "Rust GATT".into(),
            services: Vec::new(),
            advertising: None,
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
    /// Replace the default name-only advertising payload. The GAP Device Name
    /// remains controlled by name(). Oversized payloads fail in start().
    pub fn advertising_data(mut self, data: AdvertisingData) -> Self {
        self.advertising = Some(data);
        self
    }
    /// Advertise a service UUID. On first use replaces default name advertising
    /// with Flags + UUID; an explicitly configured payload is extended instead.
    /// Does not register a GATT service. Call service() separately.
    pub fn advertise_service_uuid(mut self, uuid: Uuid) -> Self {
        self.advertising = Some(self.advertising.unwrap_or_default().service_uuid(uuid));
        self
    }
    pub fn start(self) -> Result<GattServer, Error> {
        Ok(GattServer {
            runtime: btstack_core::Runtime::start_with_advertising(
                self.transport,
                self.name,
                self.services,
                self.advertising,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoIo;
    impl HciTransport for NoIo {
        fn send(&mut self, _: u8, _: &[u8]) -> Result<(), Error> {
            panic!("validation must precede HCI I/O")
        }
        fn receive(
            &mut self,
            _: std::time::Duration,
        ) -> Result<Option<btstack_core::HciPacket>, Error> {
            panic!("validation must precede HCI I/O")
        }
    }
    #[test]
    fn uuid_opt_in_replaces_default_name_but_preserves_gap_name() {
        let builder = GattServer::builder(NoIo)
            .name("GAP name")
            .advertise_service_uuid([1; 16]);
        assert_eq!(builder.name, "GAP name");
        assert_eq!(builder.advertising.unwrap().to_bytes().unwrap().len(), 21);
        assert!(GattServer::builder(NoIo).advertising.is_none());
    }
    #[test]
    fn explicit_payload_is_extended_and_overflow_fails_before_io() {
        let builder = GattServer::builder(NoIo)
            .advertising_data(AdvertisingData::new().local_name("123456789"))
            .advertise_service_uuid([1; 16])
            .service(
                GattService::new([1; 16]).characteristic(GattCharacteristic::new([2; 16]).notify()),
            );
        let error = builder
            .start()
            .err()
            .expect("oversized advertising must fail");
        assert!(error.to_string().contains("31 bytes"));
    }
}
