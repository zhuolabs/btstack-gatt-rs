package com.example.gattperipheral

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setContent { MaterialTheme { GattPeripheralRoute() } }
    }
}

@Composable
private fun GattPeripheralRoute(model: GattPeripheralViewModel = viewModel()) {
    val lifecycleOwner = LocalLifecycleOwner.current
    val state by model.uiState.collectAsStateWithLifecycle()
    LaunchedEffect(lifecycleOwner, model) {
        // ON_STOP moves Lifecycle to CREATED; there is no State.STOPPED.
        // repeatOnLifecycle cancels AND waits for cleanup before restarting.
        lifecycleOwner.lifecycle.repeatGattWhileStarted {
            model.runWhileStarted()
        }
    }
    GattPeripheralScreen(state, model::start, model::stop)
}

@Composable
private fun GattPeripheralScreen(state: GattUiState, onStart: () -> Unit, onStop: () -> Unit) {
    Scaffold { insets ->
        Column(
            modifier = Modifier.fillMaxSize().padding(insets)
                .verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Text("Rust GATT Peripheral", style = MaterialTheme.typography.headlineMedium)
            Text("USB Bluetooth 0411:0374\nAdvertised name: Rust GATT\nNordic UART: write RX, read/notify TX")
            Text("The server pauses when you leave this screen and resumes when you return. Stop disables automatic resume.")
            Text(state.status, style = MaterialTheme.typography.bodyLarge)
            Button(onClick = onStart, enabled = !state.active && !state.permissionPending) {
                Text("Start GATT server")
            }
            OutlinedButton(onClick = onStop, enabled = state.enabled) {
                Text("Stop GATT server")
            }
            Text("GATT events: adb logcat -s BtstackGatt", style = MaterialTheme.typography.bodySmall)
        }
    }
}
