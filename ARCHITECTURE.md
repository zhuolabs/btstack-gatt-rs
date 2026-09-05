# Architecture

BTstack is pinned as a git submodule in `vendor/btstack`. Initialize it with
`git submodule update --init`. Its upstream license includes a noncommercial
restriction; see `vendor/btstack/LICENSE` for redistribution and commercial use.

## Source review (2026-09-05)

Reviewed upstream `src/hci_transport.h`, `src/btstack_run_loop.h`,
`platform/embedded/btstack_run_loop_embedded.c`, `platform/libusb/hci_transport_h2_libusb.c`,
`example/Makefile.inc`, `src/ble/att_server.h`, and `src/ble/att_db_util.h`.

* `hci_transport_t` has name, init, open, close, register_packet_handler,
  can_send_packet_now, send_packet, set_baudrate, reset_link, set_sco_config.
  The C shim includes the original headers and owns this struct; Rust does not
  replicate its layout. Transport callbacks consume borrowed packets only during
  the call. Incoming packets are owned Rust buffers delivered on the stack thread.
* USB commands use class/device control OUT (request/value/index zero); events
  use interrupt IN; ACL uses bulk IN/OUT. Discover endpoint addresses and sizes
  from interface descriptors. SCO/ISO are outside this implementation.
* nusb 0.2 provides blocking `MaybeFuture::wait`, interface `control_out`, and
  typed bulk/interrupt endpoints. Windows requires WinUSB (already installed on
  the supplied 0411:0374). Linux needs USB permissions and kernel-driver detach.
  See <https://docs.rs/nusb/latest/nusb/>.
* BTstack's embedded run loop has `execute_once` and timer processing, but its
  callback list is not a host-thread-safe queue. Only the stack thread accesses it.
  Rust channels wake the owning thread with `recv_timeout`; USB workers never
  call BTstack. Timers are serviced at least every 5 ms. No Tokio dependency.
* `cc` builds the BLE peripheral subset from `example/Makefile.inc`: memory,
  run loop, HCI, L2CAP, crypto, ATT, SM, and an in-memory LE device database.
  Build definitions are maintained in `btstack-sys/c/btstack_config.h`.

## Crates and ownership

`btstack-sys` builds C and exposes a small C ABI. `btstack-core` owns the single
stack thread, catches application callback panics at the FFI boundary, dispatches
HCI packets, and owns GATT callback storage. `btstack-nusb` implements the core
transport trait. `btstack-gatt` exposes safe service/characteristic builders and
a running server. The example composes them. Dependencies are acyclic.

Database construction uses BTstack's `att_db_util` on the owning thread; returned
handles remain internal. UUIDs use canonical big-endian byte order. CCCDs are
maintained per connection. Notifications are queued and sent only on BTstack's
can-send-now event. Read offsets and prepared-write rejection are handled inside
the wrapper. The first version uses unencrypted attributes and volatile pairing
storage. Multiple simultaneous adapter instances are rejected because BTstack
has process-global state.

## Validation milestones

1. Workspace and C build.
2. USB descriptor discovery and exclusive interface claim.
3. HCI reset/initialization via nusb and ACL transport.
4. Controller-confirmed LE advertising with an initialized ATT server.
5. Safe API, shutdown, tests, and documented actual hardware results.

Over-the-air discovery and a real Central's read/write/notification checks must
be distinguished from controller command-complete evidence.
