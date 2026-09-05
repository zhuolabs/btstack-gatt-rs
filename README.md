# btstack-gatt-rs

An initial implementation of a USB BLE GATT Peripheral in Rust using BlueKitchen
BTstack. The Bluetooth protocol stack runs in BTstack C, while the USB transport
uses Rust's `nusb`. The server does not use libusb, WinRT, or BlueZ.

Verified on Windows with a USB dongle with **VID 0411 / PID 0374**.
A separate built-in Bluetooth adapter was used as a Central to verify discovery,
connections, reads, writes, notifications, and reconnection over the air.
See the [hardware validation record](docs/VALIDATION.md) and
[architecture](ARCHITECTURE.md).

## Build and run

On Windows, install the Rust MSVC toolchain and Visual Studio C++ Build Tools.
For a fresh clone, use `git clone --recurse-submodules`.

```powershell
git submodule update --init
cargo build --workspace
cargo run -p gatt-peripheral
```

The example opens `0411:0374` and advertises as `Rust GATT`.
Press `Ctrl+C` to stop advertising, close connections, and release the USB interface.
To run for a fixed duration or only probe the USB device:

```powershell
cargo run -p gatt-peripheral -- --seconds 30
cargo run -p gatt-peripheral -- --probe
```

Key startup messages:

```text
USB 0411:0374, interface 0, event IN 0x81, ACL IN 0x82, ACL OUT 0x02
BTstack HCI_STATE_WORKING
GATT server ready: LE advertising enabled (controller status 0x00)
```

After configuring the ATT server, `start()` waits for the controller to confirm
that advertising has been enabled successfully. Startup failures and timeouts
return `Err`. Shutdown waits for `HCI_STATE_OFF`.

To enable HCI tracing:

```powershell
$env:BTSTACK_HCI_TRACE = '1'
cargo run -p gatt-peripheral -- --seconds 10
Remove-Item Env:BTSTACK_HCI_TRACE
```

On Windows, the target dongle requires **WinUSB**. It was already installed on
the tested dongle; no driver changes were made.
Linux uses `detach_and_claim_interface` and requires USB access permissions.
Linux hardware has not been tested.

## Exposed services

The example uses Nordic UART Service UUIDs, with Read support added to TX.

| Item | UUID | Behavior |
|---|---|---|
| Service | `6e400001-b5a3-f393-e0a9-e50e24dcca9e` | Custom service |
| RX | `6e400002-b5a3-f393-e0a9-e50e24dcca9e` | Write / Write Without Response; logs received data |
| TX | `6e400003-b5a3-f393-e0a9-e50e24dcca9e` | Reads return `hello from Rust`; subscribed peers receive a `tick N` notification every second |

The GAP Device Name characteristic and GATT service are also registered.
Advertisements contain the local name and Flags. Discover the peripheral by name;
the advertisements do not include the 128-bit service UUID.

## API

Pass services and characteristics to `GattServer::builder(transport)`, then call
`.start()?` to start the dedicated thread.
See the [example](examples/gatt-peripheral/src/main.rs).

- `on_read` returns the entire value. ATT length queries and offsets are handled internally.
- `on_write` receives the connection and incoming value. Prepared writes and nonzero offsets are rejected.
- `ServerEvent` reports connections, disconnections, and subscription changes. CCCDs are managed per connection.
- `server.notify(&connection, characteristic_uuid, data)` reports successful enqueueing.
  It returns an error for unsubscribed or disconnected peers, values exceeding the
  negotiated MTU, or a full queue. Subsequent transmission failures are reported as events.
- Callbacks execute on the BTstack thread. Keep them short and do not call blocking
  server methods from them. Panics are caught at the FFI boundary and converted to ATT errors.

## Validation

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Stop the example before running the hardware test, which restarts the server
within a single process:

```powershell
cargo test -p gatt-peripheral --test hardware -- --ignored --nocapture
```

If a separate OS-managed Bluetooth adapter is available, run the following while
the example is running. `uv` installs the validation script's Python dependencies
in an isolated environment. The Rust library does not require Python.

```powershell
uv run scripts/verify_gatt.py --address 08:BE:AC:47:46:F7 --rounds 2
```

Only this Central verification script uses OS Bluetooth APIs. It connects to the
server matching the specified address and name, verifies service discovery,
reads, writes, notifications, unsubscribe, and disconnect, then reconnects.
It does not connect to other Bluetooth devices.

## Initial scope

One active BTstack runtime per process, with at most 16 services and 64 characteristics.
Supports 128-bit UUIDs, attributes requiring no encryption, and Read / Write / Notify.
Per-connection subscription management is unit-tested; simultaneous connections
from multiple physical Centrals have not been verified.
BTstack's ATT database buffer is reused within the process.

Indications, arbitrary descriptors, a public permissions API, persistent bonding,
Secure Connections, prepared/execute writes, async APIs, and Classic/SCO/ISO remain
future work. The application API does not expose raw pointers or ATT handles.

## Licensing

BTstack is subject to its upstream license, which includes a noncommercial
restriction. For commercial use, review the [BTstack license](vendor/btstack/LICENSE)
and BlueKitchen's terms. Dependencies retain their respective licenses.
