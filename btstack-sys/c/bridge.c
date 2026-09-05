/* BTstack structs stay in C, compiled against the pinned upstream headers. */
#include "btstack.h"
#include "btstack_run_loop_embedded.h"
#include "ble/att_db_util.h"
#include <string.h>

static int (*rust_send)(uint8_t, const uint8_t *, uint16_t);
static uint32_t (*rust_time)(void);
static void (*rust_event)(uint8_t, uint16_t, uint16_t);
static void (*transport_handler)(uint8_t, uint8_t *, uint16_t);
static btstack_packet_callback_registration_t registration;
static uint8_t advertising[31];
static int advertising_requested;
static struct {
    btstack_context_callback_registration_t registration;
    uint16_t handle;
    int busy;
} send_requests[16];

uint32_t hal_time_ms(void) { return rust_time(); }
void hal_cpu_disable_irqs(void) {}
void hal_cpu_enable_irqs(void) {}
void hal_cpu_enable_irqs_and_sleep(void) {}
static int transport_open(void) { return 0; }
static int transport_close(void) { return 0; }
static void register_handler(void (*handler)(uint8_t, uint8_t *, uint16_t)) { transport_handler = handler; }
static int send_packet(uint8_t type, uint8_t *data, int size) { return rust_send(type, data, (uint16_t)size); }
/* NULL can_send_packet_now explicitly selects BTstack's synchronous transport contract. */
static const hci_transport_t transport = {
    .name = "nusb", .open = transport_open, .close = transport_close,
    .register_packet_handler = register_handler, .send_packet = send_packet
};

static void event_handler(uint8_t type, uint16_t channel, uint8_t *packet, uint16_t size) {
    (void)channel;
    if (type != HCI_EVENT_PACKET || size < 2) return;
    switch (packet[0]) {
        case BTSTACK_EVENT_STATE:
            if (btstack_event_state_get_state(packet) == HCI_STATE_WORKING) rust_event(1, 0, 0);
            break;
        case HCI_EVENT_COMMAND_COMPLETE:
            if (size >= 6) {
                uint16_t opcode = little_endian_read_16(packet, 3);
                if (opcode == 0x200a && advertising_requested) rust_event(2, 0, packet[5]);
                if (packet[5] != 0) rust_event(6, opcode, packet[5]);
            }
            break;
        case HCI_EVENT_DISCONNECTION_COMPLETE:
            rust_event(4, hci_event_disconnection_complete_get_connection_handle(packet), 0);
            break;
        case ATT_EVENT_CONNECTED:
            rust_event(3, att_event_connected_get_handle(packet), att_server_get_mtu(att_event_connected_get_handle(packet)));
            break;
        case ATT_EVENT_MTU_EXCHANGE_COMPLETE:
            rust_event(7, att_event_mtu_exchange_complete_get_handle(packet), att_event_mtu_exchange_complete_get_MTU(packet));
            break;
        default: break;
    }
}

void rs_init(int (*send)(uint8_t, const uint8_t *, uint16_t), uint32_t (*time_ms)(void),
             void (*event)(uint8_t, uint16_t, uint16_t)) {
    rust_send = send; rust_time = time_ms; rust_event = event;
    advertising_requested = 0;
    memset(send_requests, 0, sizeof(send_requests));
    btstack_memory_init();
    btstack_run_loop_init(btstack_run_loop_embedded_get_instance());
    hci_init(&transport, NULL);
    l2cap_init();
    le_device_db_init();
    sm_init();
    sm_set_io_capabilities(IO_CAPABILITY_NO_INPUT_NO_OUTPUT);
    sm_set_authentication_requirements(0);
    memset(&registration, 0, sizeof(registration));
    registration.callback = event_handler;
    hci_add_event_handler(&registration);
    att_db_util_init();
}
void rs_service(const uint8_t *uuid) { att_db_util_add_service_uuid128(uuid); }
uint16_t rs_characteristic(const uint8_t *uuid, uint16_t properties) {
    return att_db_util_add_characteristic_uuid128(uuid, properties | ATT_PROPERTY_DYNAMIC,
        ATT_SECURITY_NONE, ATT_SECURITY_NONE, NULL, 0);
}
void rs_gap_name(const uint8_t *name, uint16_t len) {
    att_db_util_add_service_uuid16(0x1800);
    att_db_util_add_characteristic_uuid16(0x2a00, ATT_PROPERTY_READ, ATT_SECURITY_NONE,
        ATT_SECURITY_NONE, (uint8_t *)name, len);
    att_db_util_add_service_uuid16(0x1801);
}
int rs_start(att_read_callback_t read, att_write_callback_t write, const uint8_t *adv, uint8_t len) {
    att_server_init(att_db_util_get_address(), read, write);
    att_server_register_packet_handler(event_handler);
    memcpy(advertising, adv, len);
    bd_addr_t zero = {0};
    gap_advertisements_set_params(0x00a0, 0x00a0, 0, 0, zero, 7, 0);
    gap_advertisements_set_data(len, advertising);
    advertising_requested = 1;
    gap_advertisements_enable(1);
    return hci_power_control(HCI_POWER_ON);
}
void rs_receive(uint8_t type, uint8_t *packet, uint16_t len) { transport_handler(type, packet, len); }
void rs_poll(void) { btstack_run_loop_embedded_execute_once(); }
static void can_send(void *context) {
    unsigned i = (unsigned)(uintptr_t)context;
    send_requests[i].busy = 0;
    rust_event(5, send_requests[i].handle, 0);
}
void rs_request_send(uint16_t connection) {
    unsigned i;
    for (i = 0; i < 16; i++) if (send_requests[i].busy && send_requests[i].handle == connection) return;
    for (i = 0; i < 16; i++) {
        if (send_requests[i].busy) continue;
        send_requests[i].busy = 1;
        send_requests[i].handle = connection;
        send_requests[i].registration.callback = can_send;
        send_requests[i].registration.context = (void *)(uintptr_t)i;
        if (att_server_request_to_send_notification(&send_requests[i].registration, connection)) send_requests[i].busy = 0;
        return;
    }
}
uint8_t rs_notify(uint16_t connection, uint16_t attribute, const uint8_t *data, uint16_t len) {
    return att_server_notify(connection, attribute, data, len);
}
void rs_stop(void) { advertising_requested = 0; hci_power_control(HCI_POWER_OFF); }
void rs_deinit(void) {
    att_server_deinit(); sm_deinit(); l2cap_deinit(); hci_deinit();
    btstack_run_loop_deinit(); btstack_memory_deinit();
    transport_handler = NULL;
}
