use crate::{Error, Uuid};

/// Legacy advertising payload (31 bytes maximum). Defaults to Flags=0x06
/// (LE General Discoverable, BR/EDR Not Supported), without a local name.
/// Services registered in GATT are not automatically advertised.
#[derive(Clone, Debug, Default)]
pub struct AdvertisingData {
    name: Option<String>,
    services: Vec<Uuid>,
}

impl AdvertisingData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Include a Complete Local Name. This does not change the GAP Device Name.
    pub fn local_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Include a canonical big-endian UUID in the Complete List of 128-bit
    /// Service UUIDs. Duplicates are ignored. One UUID fits legacy advertising;
    /// a second distinct UUID causes validation to fail instead of truncating.
    pub fn service_uuid(mut self, uuid: Uuid) -> Self {
        if !self.services.contains(&uuid) {
            self.services.push(uuid);
        }
        self
    }

    /// Encode AD structures, validating the byte limit before controller startup.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let name_len = match &self.name {
            Some(name) if name.is_empty() => {
                return Err("Advertising local name must not be empty".into());
            }
            Some(name) => name
                .len()
                .checked_add(2)
                .ok_or("Advertising data too large")?,
            None => 0,
        };
        let services_len = if self.services.is_empty() {
            0
        } else {
            self.services
                .len()
                .checked_mul(16)
                .and_then(|n| n.checked_add(2))
                .ok_or("Advertising data too large")?
        };
        let length = 3usize
            .checked_add(name_len)
            .and_then(|n| n.checked_add(services_len))
            .ok_or("Advertising data too large")?;
        if length > 31 {
            return Err("Legacy advertising data exceeds 31 bytes".into());
        }
        let mut data = vec![2, 1, 6];
        if !self.services.is_empty() {
            data.extend([(services_len - 1) as u8, 0x07]);
            for uuid in &self.services {
                data.extend(uuid.iter().rev());
            }
        }
        if let Some(name) = &self.name {
            data.extend([(name.len() + 1) as u8, 0x09]);
            data.extend(name.as_bytes());
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_and_uuid_use_bluetooth_byte_order() {
        let uuid = [
            0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255,
        ];
        assert_eq!(AdvertisingData::new().to_bytes().unwrap(), [2, 1, 6]);
        assert_eq!(
            AdvertisingData::new()
                .service_uuid(uuid)
                .to_bytes()
                .unwrap(),
            [
                2, 1, 6, 17, 7, 255, 238, 221, 204, 187, 170, 153, 136, 119, 102, 85, 68, 51, 34,
                17, 0
            ]
        );
    }
    #[test]
    fn byte_limits_are_exact_and_never_truncate() {
        assert_eq!(
            AdvertisingData::new()
                .local_name("a".repeat(26))
                .to_bytes()
                .unwrap()
                .len(),
            31
        );
        assert!(
            AdvertisingData::new()
                .local_name("a".repeat(27))
                .to_bytes()
                .is_err()
        );
        assert!(AdvertisingData::new().local_name("").to_bytes().is_err());
        let uuid = AdvertisingData::new().service_uuid([1; 16]);
        assert_eq!(
            uuid.clone()
                .local_name("12345678")
                .to_bytes()
                .unwrap()
                .len(),
            31
        );
        assert!(uuid.clone().local_name("123456789").to_bytes().is_err());
        assert!(uuid.clone().local_name("あいう").to_bytes().is_err());
        assert_eq!(
            uuid.clone().service_uuid([1; 16]).to_bytes().unwrap().len(),
            21
        );
        assert!(uuid.service_uuid([2; 16]).to_bytes().is_err());
    }
}
