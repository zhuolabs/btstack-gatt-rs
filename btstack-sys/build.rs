fn main() {
    let root = "../vendor/btstack";
    let mut build = cc::Build::new();
    build
        .include("c")
        .include(format!("{root}/src"))
        .include(format!("{root}/platform/embedded"));
    for file in [
        "src/btstack_memory.c",
        "src/btstack_linked_list.c",
        "src/btstack_memory_pool.c",
        "src/btstack_run_loop.c",
        "src/btstack_util.c",
        "src/hci.c",
        "src/hci_cmd.c",
        "src/hci_dump.c",
        "src/hci_event.c",
        "src/hci_event_builder.c",
        "src/l2cap.c",
        "src/l2cap_signaling.c",
        "src/btstack_tlv.c",
        "src/btstack_crypto.c",
        "src/ble/att_db.c",
        "src/ble/att_db_util.c",
        "src/ble/att_dispatch.c",
        "src/ble/att_server.c",
        "src/ble/sm.c",
        "src/ble/le_device_db_memory.c",
        "platform/embedded/btstack_run_loop_embedded.c",
    ] {
        build.file(format!("{root}/{file}"));
    }
    build.file("c/bridge.c").warnings(false).compile("btstack");
    println!("cargo:rerun-if-changed=c");
    println!("cargo:rerun-if-changed=../vendor/btstack");
}
