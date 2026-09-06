package com.example.gattperipheral

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

// BTstack is process-global. Serialize different Lifecycle Owners too, until
// the previous owner's NonCancellable native/USB cleanup has finished.
private val gattSession = Mutex()

internal suspend fun Lifecycle.repeatGattWhileStarted(block: suspend () -> Unit) {
    repeatOnLifecycle(Lifecycle.State.STARTED) {
        gattSession.withLock { block() }
    }
}
