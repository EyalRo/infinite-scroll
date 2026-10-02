package art.infinitescroll.control.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.os.ParcelUuid
import art.infinitescroll.protocol.Wire
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow

/** Emits each Infinite Scroll Pi seen while collected (filtered by service UUID). */
@SuppressLint("MissingPermission") // BLUETOOTH_SCAN is requested by MainActivity first.
fun scanForInstallation(adapter: BluetoothAdapter): Flow<BluetoothDevice> = callbackFlow {
    val scanner = adapter.bluetoothLeScanner ?: run { close(IllegalStateException("Bluetooth is off")); return@callbackFlow }
    val callback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) { trySend(result.device) }
        override fun onScanFailed(errorCode: Int) { close(IllegalStateException("scan failed ($errorCode)")) }
    }
    val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(Wire.SERVICE)).build()
    scanner.startScan(listOf(filter), ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build(), callback)
    awaitClose { scanner.stopScan(callback) }
}
