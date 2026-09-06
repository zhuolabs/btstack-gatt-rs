# Validation record — 2026-09-06 (JST)

## Environment

- Windows / `x86_64-pc-windows-msvc`
- Rust 1.95.0, Cargo 1.95.0; Visual Studio 2022 Build Tools, MSVC 14.44
- BTstack submodule: `431d58d5613fd8fae38afe50282b25302de84bf7`
- nusb 0.2.7; all Rust dependencies pinned in Cargo.lock
- Peripheral: USB `0411:0374`, WinUSB, interface 0
- Discovered endpoints: interrupt IN `0x81`, bulk IN `0x82`, bulk OUT `0x02`
- Controller public address: `08:BE:AC:47:46:F7`
- Read Local Version: HCI 0x0a, manufacturer 0x005d, LMP subversion 0x8761
- Independent Central: PC's OS-managed Realtek adapter `0bda:4853`, Bleak 3.0.1 / WinRT

No driver replacement or firmware installation was performed.

## Controller evidence

Executed `cargo run -p gatt-peripheral -- --seconds 5` with `BTSTACK_HCI_TRACE=1`.
Representative HCI packets (without H4 type prefix):

```text
TX command: 03 0c 00                  HCI Reset
RX event:   0e 04 03 03 0c 00         Command Complete, success
...
BTstack HCI_STATE_WORKING
TX command: 06 20 0f ...              LE Set Advertising Parameters
RX event:   0e 04 02 06 20 00         success
TX command: 08 20 20 0e 02 01 06 ...  LE Set Advertising Data (Rust GATT)
RX event:   0e 04 02 08 20 00         success
TX command: 0a 20 01 01               LE Set Advertising Enable
RX event:   0e 04 02 0a 20 00         success
GATT server ready: LE advertising enabled (controller status 0x00)
```

The final shutdown implementation pumps USB events and BTstack timers until
`HCI_STATE_OFF`, then drops the transport and joins the USB worker.

## Real over-the-air GATT checks

`uv run scripts/verify_gatt.py` uses the second radio to test the actual server.
The following passed twice, including reconnection to the same running server:

```text
SCAN PASS: Rust GATT (08:BE:AC:47:46:F7)
SERVICE DISCOVERY PASS
READ PASS: bytearray(b'hello from Rust')
WRITE REQUEST + COMMAND PASS
NOTIFY PASS: b'tick ...'
UNSUBSCRIBE PASS
DISCONNECT PASS
```

Server-side logs independently show both payloads arriving via ACL/ATT, subscription
changes, and disconnect. Reconnected peers receive a new connection ID.
No smartphone operation or manual user intervention was needed.

The scanner waits for both address and advertising local name: Windows sometimes
delivers an empty scan response before the advertisement containing the name.

## Automated checks

- `cargo test --workspace`: six tests pass. Covers read snapshot/offset behavior,
  per-peer CCCDs, invalid CCCD values, prepared/offset writes without callback side
  effects, callback panic containment, HCI framing, and builder/advertising limits.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo fmt --all -- --check`: pass.
- `cargo test -p gatt-peripheral --test hardware -- --ignored --nocapture`: pass.
  Starts and shuts down the actual controller twice in one process, verifying
  advertising success, C lifecycle reset, and USB release/reclaim.

## Remaining validation

Linux/macOS builds and hardware; simultaneous multiple Centrals; forced USB removal
during traffic; long-duration stress/throughput; pairing/encryption. These are not
claimed as verified by the startup and GATT smoke tests above.

## USB selection and Android FD support — 2026-09-06

- `--vid 0411 --did 0374 --seconds 2`: actual Windows controller startup,
  advertising confirmation, and shutdown pass (`--did` aliases `--pid`).
- Host workspace tests: nine pass, including three CLI tests for hexadecimal IDs,
  missing/duplicate/invalid arguments, and default selection.
- Android workspace cross-build passes for arm64-v8a, armeabi-v7a, x86_64, and x86,
  using NDK 27.0.12077973 and API level 23.
- Android ARM64 `cargo test --workspace --no-run` builds and links the tests,
  including non-USB FD rejection, preservation of borrowed FDs on failure, and
  rejection of desktop enumeration on Android. These Android tests were compiled,
  not executed; no Android device was attached.
- Android hardware was unverified at this stage; see the subsequent app validation below.
- Windows and Android ARM64 Clippy pass with warnings denied. The Android TLS
  macro expansion produces a spurious `missing_const_for_thread_local` lint even
  with const initializers; its allowance is scoped to the TLS declarations on
  Android. Rust examples in `docs/ANDROID.md` also type-check for Android ARM64.

## Android UniFFI app — 2026-09-06

`examples/gatt-peripheral-android` was created using official Android CLI
1.0.16261425 (`android init`, `android create empty-activity`). Tested with
UniFFI 0.31.0, AGP 9.0.1, Gradle 9.1.0, NDK 27.0.12077973, API 26 / ARM64.

Device: Pixel 9a, wireless adb, USB Host dongle `0411:0374` (Realtek Bluetooth
Radio), controller address `08:BE:AC:47:46:F7`. PC central: Windows-managed
Realtek Bluetooth Adapter. No driver changes or pairing were needed.

- Installed/launched the debug APK with `android run`, selected Start and accepted
  the normal USB permission dialog through the UI.
- USB FD duplication, interface claim, `HCI_STATE_WORKING` and advertising enable
  status `0x00` were confirmed in `adb logcat -s BtstackGatt`.
- The unmodified `uv run scripts/verify_gatt.py --rounds 2` passed all checks in
  two successive rounds: scan, service discovery, read, write request/command,
  notify, unsubscribe, disconnect and reconnect.
- logcat showed `on_read TX`, both write payloads, and `SubscriptionChanged`
  Notify/None for each connection.
- Stop button cancelled the Kotlin coroutine; logcat showed `HCI_STATE_OFF`,
  `GATT server stopped; USB interface released`, then Kotlin connection cleanup.
  Restarting in the same process successfully reclaimed the controller.
- Leaving the activity with Home also stopped the server and released USB.
  Returning/restarting passed another two-round PC test.
- Host tests: 11 passed, one desktop hardware test intentionally ignored because
  the dongle was attached to Android. New tests exercise native future cancellation
  and stop-before-first-poll cleanup. Host/Android ARM64 Clippy and Android
  `assembleDebug lintDebug` passed; lint advisory warnings remain.
- Final APK (Android-specific UniFFI bindings enabled): after the user's other BLE
  client disconnected, the two-round PC test passed again at 15:24 JST, including
  notifications `tick 100` and `tick 106`. Server logcat recorded connection IDs
  3 and 4 and both write payloads. The app was left running for further testing.

Initial observation: the first PC connection timed out; a subsequent scan missed
the advertisement. Stop/Start recovered operation, followed by successful runs.
The initial failure's cause remains unknown. Physical USB detach during traffic,
older Android versions, other phone models and long-duration operation were not tested.

## Compose Lifecycle revision — 2026-09-06

- Previous View-based implementation checkpoint: `11f67b5`.
- Compose Material 3 screen with LocalLifecycleOwner, lifecycle-aware UI state,
  and repeatOnLifecycle(STARTED) for the server session.
- Pixel 9a: Home caused HCI_STATE_OFF and USB release; returning through the
  launcher automatically started advertising. The PC script then passed two
  rounds, including read/write/notify, unsubscribe and reconnect (15:36 JST).
- Explicit Stop remained stopped after Home/return (`resume enabled=false`).
  Start re-enabled the session.
- An initial rapid switch to a separate Activity reproduced USB `errno 16` with
  a per-ViewModel lock. After moving serialization to a process-wide lifecycle
  gate, the same switch released the previous USB connection before the next
  owner advertised successfully (15:37 JST).
- Two JVM tests exercise STOP/immediate START and replacement of Lifecycle Owner,
  including delayed non-cancellable cleanup. `testDebugUnitTest`, `assembleDebug`
  and `lintDebug` passed. Existing Rust code was unchanged.
