package com.example.gattperipheral

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import kotlinx.coroutines.*

class MainActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var usb: UsbManager
    private lateinit var status: TextView
    private lateinit var start: Button
    private lateinit var stop: Button
    private var serverJob: Job? = null
    private var selected: UsbDevice? = null
    private var permissionPending = false
    private val permissionAction get() = "$packageName.USB_PERMISSION"

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                permissionAction -> {
                    if (!permissionPending) return
                    permissionPending = false
                    val device = selected
                    if (device != null && usb.hasPermission(device)) launchServer(device)
                    else showStatus("USB permission denied. Tap Start to retry.")
                    updateButtons()
                }
                UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                    @Suppress("DEPRECATION")
                    val device = intent.getParcelableExtra<UsbDevice>(UsbManager.EXTRA_DEVICE)
                    if (device?.deviceName == selected?.deviceName) {
                        permissionPending = false
                        serverJob?.cancel(CancellationException("USB detached"))
                        selected = null
                        showStatus("USB dongle detached")
                        updateButtons()
                    }
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        usb = getSystemService(UsbManager::class.java)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 120, 32, 32)
        }
        layout.addView(TextView(this).apply { text = "Rust GATT Peripheral"; textSize = 26f })
        layout.addView(TextView(this).apply {
            text = "USB Bluetooth 0411:0374\nAdvertised name: Rust GATT\nNordic UART: write RX, read/notify TX\nKeep this screen open while testing."
            textSize = 16f
        })
        status = TextView(this).apply { text = "Ready"; textSize = 18f; setPadding(0, 40, 0, 40) }
        layout.addView(status)
        start = Button(this).apply { text = "Start GATT server"; setOnClickListener { requestStart() } }
        stop = Button(this).apply {
            text = "Stop GATT server"
            setOnClickListener {
                serverJob?.cancel(CancellationException("Stop button"))
                showStatus("Stopping…")
                updateButtons()
            }
        }
        layout.addView(start)
        layout.addView(stop)
        setContentView(layout)
        val filter = IntentFilter(permissionAction).apply { addAction(UsbManager.ACTION_USB_DEVICE_DETACHED) }
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(receiver, filter, RECEIVER_NOT_EXPORTED)
        else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            registerReceiver(receiver, filter)
        }
        updateButtons()
    }

    private fun requestStart() {
        if (serverJob != null || permissionPending) return
        val matches = usb.deviceList.values.filter { it.vendorId == 0x0411 && it.productId == 0x0374 }
        if (matches.size != 1) {
            showStatus("Expected one USB dongle 0411:0374; found ${matches.size}")
            return
        }
        val device = matches.single()
        selected = device
        if (usb.hasPermission(device)) launchServer(device)
        else {
            permissionPending = true
            showStatus("Waiting for USB permission…")
            updateButtons()
            val pending = PendingIntent.getBroadcast(this, 0, Intent(permissionAction).setPackage(packageName),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            usb.requestPermission(device, pending)
        }
    }

    private fun launchServer(device: UsbDevice) {
        if (serverJob != null) return
        serverJob = scope.launch(start = CoroutineStart.LAZY) {
            try {
                val connection = checkNotNull(usb.openDevice(device)) { "Cannot open USB device" }
                try {
                    showStatus("Server session active. Advertising/startup details: logcat BtstackGatt")
                    runGattServer(connection)
                } finally {
                    connection.close()
                }
                showStatus("Server stopped")
            } catch (cancelled: CancellationException) {
                showStatus("Stopped; USB connection released")
                throw cancelled
            } catch (error: Exception) {
                Log.e("BtstackGatt", "Server error", error)
                showStatus("Error: ${error.message}")
            } finally {
                serverJob = null
                updateButtons()
            }
        }
        serverJob!!.start()
        updateButtons()
    }

    private fun showStatus(message: String) {
        status.text = message
        Log.i("BtstackGatt", message)
    }

    private fun updateButtons() {
        start.isEnabled = serverJob == null && !permissionPending
        stop.isEnabled = serverJob != null && serverJob?.isCancelled == false
    }

    override fun onStop() {
        permissionPending = false
        serverJob?.cancel(CancellationException("Activity stopped"))
        updateButtons()
        super.onStop()
    }

    override fun onDestroy() {
        unregisterReceiver(receiver)
        scope.cancel()
        super.onDestroy()
    }
}
