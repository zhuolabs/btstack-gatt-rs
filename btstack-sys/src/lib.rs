//! Internal C ABI. All functions must be called from the single BTstack thread.

pub type ReadCallback = unsafe extern "C" fn(u16, u16, u16, *mut u8, u16) -> u16;
pub type WriteCallback = unsafe extern "C" fn(u16, u16, u16, u16, *mut u8, u16) -> i32;
unsafe extern "C" {
    pub fn rs_init(
        send: unsafe extern "C" fn(u8, *const u8, u16) -> i32,
        time: unsafe extern "C" fn() -> u32,
        event: unsafe extern "C" fn(u8, u16, u16),
    );
    pub fn rs_service(uuid: *const u8);
    pub fn rs_characteristic(uuid: *const u8, properties: u16) -> u16;
    pub fn rs_gap_name(name: *const u8, len: u16);
    pub fn rs_start(read: ReadCallback, write: WriteCallback, adv: *const u8, len: u8) -> i32;
    pub fn rs_receive(kind: u8, data: *mut u8, len: u16);
    pub fn rs_poll();
    pub fn rs_request_send(connection: u16);
    pub fn rs_notify(connection: u16, attribute: u16, data: *const u8, len: u16) -> u8;
    pub fn rs_stop();
    pub fn rs_deinit();
}
