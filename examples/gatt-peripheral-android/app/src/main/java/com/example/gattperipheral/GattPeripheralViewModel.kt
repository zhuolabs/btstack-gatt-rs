package com.example.gattperipheral

import android.app.Application
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbManager
import android.os.Build
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.SavedStateHandle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update

data class GattUiState(
    val status: String = "Ready",
    val enabled: Boolean = true,
    val active: Boolean = false,
    val permissionPending: Boolean = false,
)

class GattPeripheralViewModel(application: Application, private val savedState: SavedStateHandle) :
    AndroidViewModel(application) {
    private val usb = application.getSystemService(UsbManager::class.java)
    private val enabled = savedState.getStateFlow("serverEnabled", true)
    private val usbChanges = MutableStateFlow(0)
    private val mutableUiState = MutableStateFlow(GattUiState(enabled = enabled.value))
    val uiState = mutableUiState.asStateFlow()
    private val permissionAction = "${application.packageName}.USB_PERMISSION"
    private var permissionDevice: String? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                permissionAction -> {
                    val name = permissionDevice ?: return
                    permissionDevice = null
                    mutableUiState.update { it.copy(permissionPending = false) }
                    val device = usb.deviceList[name]
                    if (device == null || !usb.hasPermission(device)) {
                        savedState["serverEnabled"] = false
                        mutableUiState.update { it.copy(enabled = false) }
                        status("USB permission denied or device removed. Tap Start to retry.")
                    }
                    usbChanges.update { it + 1 }
                }
                UsbManager.ACTION_USB_DEVICE_ATTACHED, UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                    // Ignore unrelated USB devices so they cannot restart this server.
                    @Suppress("DEPRECATION")
                    val device = intent.getParcelableExtra<android.hardware.usb.UsbDevice>(UsbManager.EXTRA_DEVICE)
                    if (device?.vendorId != 0x0411 || device.productId != 0x0374) return
                    if (intent.action == UsbManager.ACTION_USB_DEVICE_DETACHED) {
                        permissionDevice = null
                        mutableUiState.update { it.copy(permissionPending = false) }
                    }
                    usbChanges.update { it + 1 }
                }
            }
        }
    }

    init {
        val filter = IntentFilter(permissionAction).apply {
            addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
        }
        if (Build.VERSION.SDK_INT >= 33) application.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            application.registerReceiver(receiver, filter)
        }
    }

    fun start() {
        mutableUiState.update { it.copy(enabled = true) }
        if (enabled.value) usbChanges.update { it + 1 } // Explicit retry after an error.
        else savedState["serverEnabled"] = true
    }

    fun stop() {
        savedState["serverEnabled"] = false
        mutableUiState.update { it.copy(enabled = false) }
        status(if (uiState.value.active) "Stopping…" else "Stopped")
    }

    /** Called only inside the UI Lifecycle Owner's repeatOnLifecycle(STARTED). */
    suspend fun runWhileStarted() {
        Log.i("BtstackGatt", "Lifecycle STARTED: resume enabled=${enabled.value}")
        try {
            combine(enabled, usbChanges) { requested, _ -> requested }.collectLatest { requested ->
                // collectLatest waits for the previous server's finally block.
                if (requested) runSession()
            }
        } finally {
            Log.i("BtstackGatt", "Lifecycle session ended; USB cleanup complete")
        }
    }

    private suspend fun runSession() {
        try {
            val devices = usb.deviceList.values.filter { it.vendorId == 0x0411 && it.productId == 0x0374 }
            if (devices.size != 1) {
                status("Expected one USB dongle 0411:0374; found ${devices.size}")
                return
            }
            val device = devices.single()
            if (!usb.hasPermission(device)) {
                status("Waiting for USB permission…")
                if (permissionDevice == null) {
                    permissionDevice = device.deviceName
                    mutableUiState.update { it.copy(permissionPending = true) }
                    val pending = PendingIntent.getBroadcast(getApplication(), 0,
                        Intent(permissionAction).setPackage(getApplication<Application>().packageName),
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
                    usb.requestPermission(device, pending)
                }
                return
            }
            permissionDevice = null
            mutableUiState.update { it.copy(permissionPending = false) }
            val connection = checkNotNull(usb.openDevice(device)) { "Cannot open USB device" }
            try {
                mutableUiState.update { it.copy(active = true) }
                status("Server session active. Advertising/startup details: logcat BtstackGatt")
                runGattServer(connection)
            } finally {
                connection.close()
                mutableUiState.update { it.copy(active = false) }
                status(if (enabled.value) "Paused; USB released. Resumes when STARTED." else "Stopped; USB connection released")
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            Log.e("BtstackGatt", "Server error", error)
            permissionDevice = null
            mutableUiState.update { it.copy(permissionPending = false) }
            status("Error: ${error.message}. Tap Start to retry.")
        }
    }

    private fun status(message: String) {
        mutableUiState.update { it.copy(status = message) }
        Log.i("BtstackGatt", message)
    }

    override fun onCleared() {
        getApplication<Application>().unregisterReceiver(receiver)
        super.onCleared()
    }
}
