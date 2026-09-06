package com.example.gattperipheral

import android.hardware.usb.UsbDeviceConnection
import com.example.gattperipheral.nativebridge.NativeServer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/**
 * Runs until cancellation or a native error. The caller owns connection and must
 * keep it open until this function returns; it must not claim interfaces or do USB I/O.
 */
suspend fun runGattServer(connection: UsbDeviceConnection) {
    var native: NativeServer? = null
    try {
        // Assign inside NonCancellable so cancellation cannot lose an acquired resource.
        withContext(NonCancellable + Dispatchers.IO) {
            native = NativeServer(connection.fileDescriptor)
        }
        withContext(Dispatchers.IO) { checkNotNull(native).run() }
    } finally {
        withContext(NonCancellable + Dispatchers.IO) {
            native?.let {
                try { it.stop() } finally { it.destroy() }
            }
        }
    }
}
