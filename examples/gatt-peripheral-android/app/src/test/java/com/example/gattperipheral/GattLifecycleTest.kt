package com.example.gattperipheral

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class GattLifecycleTest {
    private class Owner : LifecycleOwner {
        override val lifecycle = LifecycleRegistry.createUnsafe(this)
    }

    @Test
    fun stopThenImmediateStartWaitsForCleanup() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        try {
            val owner = Owner()
            owner.lifecycle.currentState = Lifecycle.State.CREATED
            val events = mutableListOf<String>()
            val job = launch {
                owner.lifecycle.repeatGattWhileStarted {
                    events += "start"
                    try { awaitCancellation() } finally {
                        withContext(NonCancellable) { delay(100); events += "released" }
                    }
                }
            }
            runCurrent()
            assertEquals(emptyList<String>(), events)
            owner.lifecycle.currentState = Lifecycle.State.STARTED
            runCurrent()
            assertEquals(listOf("start"), events)
            owner.lifecycle.currentState = Lifecycle.State.CREATED
            owner.lifecycle.currentState = Lifecycle.State.STARTED
            runCurrent()
            assertEquals(listOf("start"), events)
            advanceUntilIdle()
            assertEquals(listOf("start", "released", "start"), events)
            owner.lifecycle.currentState = Lifecycle.State.DESTROYED
            advanceUntilIdle()
            job.join()
            assertEquals(listOf("start", "released", "start", "released"), events)
        } finally { Dispatchers.resetMain() }
    }

    @Test
    fun newOwnerWaitsForPreviousOwnerToReleaseUsb() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        try {
            val first = Owner()
            val second = Owner()
            first.lifecycle.currentState = Lifecycle.State.STARTED
            second.lifecycle.currentState = Lifecycle.State.CREATED
            val events = mutableListOf<String>()
            val firstJob = launch {
                first.lifecycle.repeatGattWhileStarted {
                    events += "first start"
                    try { awaitCancellation() } finally {
                        withContext(NonCancellable) { delay(100); events += "first released" }
                    }
                }
            }
            runCurrent()
            val secondJob = launch {
                second.lifecycle.repeatGattWhileStarted {
                    events += "second start"
                    awaitCancellation()
                }
            }
            runCurrent()
            first.lifecycle.currentState = Lifecycle.State.CREATED
            second.lifecycle.currentState = Lifecycle.State.STARTED
            runCurrent()
            assertEquals(listOf("first start"), events)
            advanceUntilIdle()
            assertEquals(listOf("first start", "first released", "second start"), events)
            first.lifecycle.currentState = Lifecycle.State.DESTROYED
            second.lifecycle.currentState = Lifecycle.State.DESTROYED
            advanceUntilIdle()
            firstJob.join()
            secondJob.join()
        } finally { Dispatchers.resetMain() }
    }
}
