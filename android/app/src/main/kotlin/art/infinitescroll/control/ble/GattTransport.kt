package art.infinitescroll.control.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothProfile
import android.content.Context
import art.infinitescroll.protocol.Changed
import art.infinitescroll.protocol.Transport
import art.infinitescroll.protocol.Wire
import java.util.UUID
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout

enum class LinkState { DISCONNECTED, CONNECTING, CONNECTED }

class BleException(message: String) : Exception(message)

/**
 * GATT link to the Pi. Android allows one GATT operation at a time, so every
 * operation takes [opLock] and waits for its callback before the next starts.
 */
@SuppressLint("MissingPermission") // BLUETOOTH_CONNECT is requested by MainActivity before any connect.
class GattTransport(private val context: Context, private val device: BluetoothDevice) : Transport {
    override var mtu: Int = 23
        private set
    override val responseFrames = MutableSharedFlow<ByteArray>(extraBufferCapacity = 256)
    override val changed = MutableSharedFlow<Changed>(extraBufferCapacity = 16)

    private val _state = MutableStateFlow(LinkState.DISCONNECTED)
    val state: StateFlow<LinkState> = _state

    private var gatt: BluetoothGatt? = null
    private val opLock = Mutex()
    @Volatile private var pending: CompletableDeferred<Int>? = null
    @Volatile private var pendingRead: CompletableDeferred<ByteArray>? = null
    private val connectResult = CompletableDeferred<Unit>()
    private val servicesResult = CompletableDeferred<Unit>()

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                connectResult.complete(Unit)
            } else {
                // Disconnected, or a failed connection attempt.
                _state.value = LinkState.DISCONNECTED
                val failure = BleException("disconnected (status $status)")
                connectResult.completeExceptionally(failure)
                servicesResult.completeExceptionally(failure)
                pending?.completeExceptionally(failure)
                pendingRead?.completeExceptionally(failure)
                g.close()
                gatt = null
            }
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) servicesResult.complete(Unit)
            else servicesResult.completeExceptionally(BleException("service discovery failed ($status)"))
        }

        override fun onMtuChanged(g: BluetoothGatt, newMtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) mtu = newMtu
            pending?.complete(status)
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            pending?.complete(status)
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            pending?.complete(status)
        }

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) pendingRead?.complete(value)
            else pendingRead?.completeExceptionally(BleException("read failed ($status)"))
        }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            when (c.uuid) {
                Wire.RESPONSE -> responseFrames.tryEmit(value)
                Wire.CHANGED -> Changed.parse(value)?.let { changed.tryEmit(it) }
            }
        }
    }

    /** Connects, negotiates the MTU, subscribes, and verifies the protocol version. */
    suspend fun connect(): String {
        _state.value = LinkState.CONNECTING
        try {
            val g = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                ?: throw BleException("could not start a GATT connection")
            gatt = g
            withTimeout(20_000) { connectResult.await() }
            if (!g.discoverServices()) throw BleException("could not start service discovery")
            withTimeout(20_000) { servicesResult.await() }
            val service = g.getService(Wire.SERVICE) ?: throw BleException("not an Infinite Scroll device")

            operation { g.requestMtu(517) }
            val info = read(service.getCharacteristic(Wire.INFO) ?: throw BleException("missing Info characteristic"))
            for (uuid in listOf(Wire.RESPONSE, Wire.CHANGED)) {
                val characteristic = service.getCharacteristic(uuid) ?: throw BleException("missing characteristic $uuid")
                g.setCharacteristicNotification(characteristic, true)
                val cccd = characteristic.getDescriptor(CCCD) ?: throw BleException("missing CCCD for $uuid")
                operation { g.writeDescriptor(cccd, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) }
            }
            _state.value = LinkState.CONNECTED
            return info.toString(Charsets.UTF_8)
        } catch (e: Exception) {
            disconnect()
            throw e
        }
    }

    fun disconnect() {
        gatt?.disconnect()
        gatt?.close()
        gatt = null
        _state.value = LinkState.DISCONNECTED
    }

    private suspend fun operation(start: () -> Any) = opLock.withLock {
        val deferred = CompletableDeferred<Int>()
        pending = deferred
        val started = start()
        if (started == false) throw BleException("GATT operation rejected")
        if (started is Int && started != 0) throw BleException("GATT operation rejected ($started)")
        val status = withTimeout(10_000) { deferred.await() }
        if (status != BluetoothGatt.GATT_SUCCESS) throw BleException("GATT operation failed ($status)")
    }

    private suspend fun read(characteristic: BluetoothGattCharacteristic): ByteArray = opLock.withLock {
        val deferred = CompletableDeferred<ByteArray>()
        pendingRead = deferred
        if (gatt?.readCharacteristic(characteristic) != true) throw BleException("read rejected")
        withTimeout(10_000) { deferred.await() }
    }

    private suspend fun write(uuid: UUID, value: ByteArray, type: Int) {
        val g = gatt ?: throw BleException("not connected")
        val characteristic = g.getService(Wire.SERVICE)?.getCharacteristic(uuid) ?: throw BleException("missing characteristic $uuid")
        // The stack answers "busy" (201) while an earlier write is in flight; wait and retry.
        repeat(50) {
            try {
                operation { g.writeCharacteristic(characteristic, value, type) }
                return
            } catch (e: BleException) {
                if (e.message?.contains("(201)") == true) delay(20) else throw e
            }
        }
        throw BleException("write kept being rejected as busy")
    }

    override suspend fun writeRequestFrame(frame: ByteArray) =
        write(Wire.REQUEST, frame, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT)

    override suspend fun writeUploadChunk(chunk: ByteArray) =
        write(Wire.UPLOAD, chunk, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE)

    private companion object {
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    }
}
