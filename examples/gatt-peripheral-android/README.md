# Android USB GATT peripheral

Android USB Host → UniFFI 0.31 → Rust `btstack-gatt` → USB Bluetooth dongle.
The sample targets **VID 0411 / PID 0374**, Android 8.0+ (API 26), ARM64.
It does not use Android's built-in Bluetooth radio or require Bluetooth permissions.

## Setup and build (Windows)

Install the official [Android CLI](https://developer.android.com/tools/agents/android-cli/download),
a JDK (Android Studio's bundled JBR works), Rust, and Python/uv for PC verification.
This project was scaffolded with:

```powershell
android init
android create empty-activity --name="GATT Peripheral" --minSdk=26 --output=examples/gatt-peripheral-android
```

The generated project is already checked in; do not recreate it. Install the SDK
components and Rust tools if missing:

```powershell
android sdk install platforms/android-36 build-tools/36.0.0 ndk/27.0.12077973
rustup target add aarch64-linux-android
cargo install cargo-ndk --version 4.1.2 --locked
```

From the repository root:

```powershell
./examples/gatt-peripheral-android/build.ps1 -Run
```

`build.ps1` uses `JAVA_HOME` / `ANDROID_HOME` when set, otherwise the usual
Android Studio JBR and `%LOCALAPPDATA%/Android/Sdk` locations. `-Run` deploys
and opens the APK with `android run`. Select a device with `android run --device=...`
when more than one is connected. SDK licenses must have been accepted during setup.

Gradle itself works on other hosts with a configured JDK, Android SDK/NDK, Rust,
and cargo-ndk: `./gradlew assembleDebug` from this directory (not hardware-tested).
Only `arm64-v8a` is packaged by this sample.

APK: `app/build/outputs/apk/debug/app-debug.apk`.

## Run and verify

1. Attach the dongle to the Android device's USB Host/OTG port.
2. Open **GATT Peripheral** and accept USB access. The server starts automatically
   while the screen's Lifecycle Owner is STARTED (including RESUMED).
3. Keep the activity visible. Advertising startup and GATT events appear in:

   ```powershell
   adb logcat -s BtstackGatt
   ```

4. On the PC, using its OS-managed Bluetooth adapter, run from the repository root:

   ```powershell
   uv run scripts/verify_gatt.py --address 08:BE:AC:47:46:F7 --rounds 2
   ```

   This address belongs to the tested dongle. Supply your own controller's address
   when using another device. The script tests advertising, service discovery,
   read, write request, write command, notification, unsubscribe and reconnection.

5. Leaving the activity cancels the coroutine and waits for native cleanup.
   Returning to STARTED automatically opens a fresh USB connection and resumes.
   **Stop GATT server** explicitly disables automatic resume until **Start GATT
   server** is pressed; this choice survives Activity recreation via SavedStateHandle.
   Detaching the dongle also cancels the session; reattachment retries while the
   screen is STARTED and the server remains enabled. This is an activity-scoped
   example, not a background/foreground service.

## Compose and Lifecycle

`MainActivity` is a `ComponentActivity` displaying a Material 3 Compose screen.
`LocalLifecycleOwner` supplies the actual screen owner and
`collectAsStateWithLifecycle()` observes the ViewModel's UI state. A
`LaunchedEffect` keyed by that owner calls `repeatOnLifecycle(STARTED)` through
`repeatGattWhileStarted`. ON_STOP moves Lifecycle to CREATED (there is no
`Lifecycle.State.STOPPED`), cancelling the session; ON_START starts a new one.

The helper holds a process-wide Mutex because BTstack permits one active server.
This waits for the previous owner's non-cancellable USB cleanup even if an entirely
new Activity/ViewModel starts before the previous Activity finishes stopping.
Within an owner, `collectLatest` serializes explicit Start/Stop and USB changes.
USB permissions and UI state are managed by the ViewModel using the Application
context; permission results cannot start a server while its Lifecycle is stopped.

`GattLifecycleTest` uses LifecycleRegistry and virtual coroutine time to verify
STOP→immediate START and owner replacement both wait for resource release:

```powershell
./gradlew.bat testDebugUnitTest
```

The hard-coded Nordic UART service matches the desktop sample:

| Attribute | UUID | Behavior |
| --- | --- | --- |
| Service | `6e400001-b5a3-f393-e0a9-e50e24dcca9e` | Advertised name `Rust GATT` |
| RX | `6e400002-b5a3-f393-e0a9-e50e24dcca9e` | Write request / command, logs payload |
| TX | `6e400003-b5a3-f393-e0a9-e50e24dcca9e` | Read `hello from Rust`; notify `tick N` every second |

## Kotlin/Rust boundary and cancellation

The application-facing API is:

```kotlin
suspend fun runGattServer(connection: UsbDeviceConnection)
```

The caller obtains permission with `UsbManager`, opens the device and keeps the
connection alive until this function returns, closing it in `finally`. Do not
claim interfaces or issue USB transfers through the Java connection.

`NativeServer(fd)` synchronously duplicates the descriptor with
`fcntl(F_DUPFD_CLOEXEC)`, then starts a Rust worker. It never takes ownership of
Android's original descriptor. nusb claims interface 0 through the duplicate.
`NativeServer.run()` is a UniFFI-generated `suspend` method waiting on a Rust
oneshot future; GATT's blocking loop runs on the native worker without Tokio.
Dropping the future on cancellation stops and joins that worker. Kotlin also
calls the idempotent `stop()` barrier and `destroy()` in a
`NonCancellable + Dispatchers.IO` finally block, including cancellation before
the first native future poll. Only then does the lifecycle-scoped session close the connection.
Startup cancellation can wait for the library's bounded controller startup.

No Rust-to-Kotlin callbacks are used. Rust stdout/stderr are redirected through
a process-lifetime pipe to Android logcat (`BtstackGatt`); GATT handlers continue
to use `println!` / `eprintln!`. The UI shows session/error status; authoritative
advertising readiness is the `GATT server ready` log, not the session status.

Following the [UniFFI Gradle guide](https://mozilla.github.io/uniffi-rs/0.31/kotlin/gradle.html),
Gradle builds the Rust cdylib, generates Kotlin bindings before compilation,
and includes JNA's Android AAR plus coroutines. With AGP 9 the generated directory
is registered as a Kotlin source set. `uniffi.toml` enables Android-specific
cleaner handling, with AndroidX annotations. Bindings are generated from the
ARM64 library metadata by the host `uniffi-bindgen` executable from the same
pinned Rust dependency. No global uniffi-bindgen installation is needed.

## Hardware validation (2026-09-06)

Pixel 9a over wireless adb, Realtek/Buffalo `0411:0374`, Windows PC central:

- USB permission dialog, FD opening, BTstack startup and advertising: passed.
- Unmodified `scripts/verify_gatt.py --rounds 2`: passed, including reconnection.
- logcat independently recorded both write payloads and Notify/None subscriptions.
- Stop button cancellation: `HCI_STATE_OFF`, USB release, then successful restart.
- Home/activity stop: native shutdown and USB release confirmed; subsequent run
  also passed two central verification rounds.
- Compose version: Home→launcher automatically resumed without pressing Start;
  the PC script passed two rounds after resume. Explicit Stop remained disabled
  after Home→launcher. A rapid switch to a second Activity instance waited for
  the first instance's USB release before successfully advertising again.
- Compose lifecycle tests (2), `assembleDebug` and `lintDebug` passed.
- Host workspace tests (11), host and Android ARM64 Clippy, APK build and Android
  Lint passed (no lint errors; advisory warnings remain).

The first central connection timed out and the following scan did not find the
advertisement. Stop/Start recovered it; subsequent two-round runs passed. The
cause of that initial failure has not been isolated. Physical USB removal during
traffic, long runs, other phones/ABIs and older Android versions remain untested.
