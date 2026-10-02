package art.infinitescroll.control

import android.app.Application
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import art.infinitescroll.control.ble.GattTransport
import art.infinitescroll.control.ble.LinkState
import art.infinitescroll.control.ble.scanForInstallation
import art.infinitescroll.protocol.Capabilities
import art.infinitescroll.protocol.Domain
import art.infinitescroll.protocol.HistoryRecord
import art.infinitescroll.protocol.InstallationApi
import art.infinitescroll.protocol.Job
import art.infinitescroll.protocol.LibraryItem
import art.infinitescroll.protocol.Preview
import art.infinitescroll.protocol.RpcClient
import art.infinitescroll.protocol.RpcException
import art.infinitescroll.protocol.Schedule
import art.infinitescroll.protocol.Stats
import art.infinitescroll.protocol.Status
import art.infinitescroll.protocol.Uploader
import kotlinx.coroutines.Job as CoroutineJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** What the user is told. ACCEPTED means the Pi now owns the work; COMPLETED means it is done. */
enum class NoticeKind { ACCEPTED, COMPLETED, ERROR }
data class Notice(val kind: NoticeKind, val text: String, val id: Long = System.nanoTime())

data class UiState(
    val link: LinkState = LinkState.DISCONNECTED,
    val scanning: Boolean = false,
    val devices: List<BluetoothDevice> = emptyList(),
    val status: Status? = null,
    val library: List<LibraryItem> = emptyList(),
    val jobs: List<Job> = emptyList(),
    val history: List<HistoryRecord> = emptyList(),
    val stats: Stats? = null,
    val schedule: Schedule? = null,
    val preview: Preview? = null,
    val capabilities: Capabilities? = null,
    /** Live values read from the printer (e.g. `darkness`); empty if it could not be reached. */
    val printerValues: Map<String, Long> = emptyMap(),
    val printerSettingsError: String? = null,
    /** Library thumbnails by item id, fetched lazily as items scroll into view. */
    val thumbnails: Map<String, ImageBitmap> = emptyMap(),
    val uploadProgress: Float? = null,
    val notice: Notice? = null,
)

/**
 * Presentation state only. Every operation is a request to the Pi; nothing
 * here prints, schedules, converts images or stores a catalog.
 */
class InstallationViewModel(app: Application) : AndroidViewModel(app) {
    private val adapter = (app.getSystemService(BluetoothManager::class.java)).adapter
    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    private var transport: GattTransport? = null
    private var rpc: RpcClient? = null
    private var api: InstallationApi? = null
    private var scanJob: CoroutineJob? = null
    private var linkJobs = mutableListOf<CoroutineJob>()
    private val thumbnailsRequested = mutableSetOf<String>()

    private fun notice(kind: NoticeKind, text: String) = _state.update { it.copy(notice = Notice(kind, text)) }
    fun dismissNotice() = _state.update { it.copy(notice = null) }

    private fun describe(e: Throwable) = when (e) {
        is RpcException -> when (e.code) {
            "unavailable" -> "A service on the Pi is not running: ${e.message}"
            "unsupported" -> "Not supported by this installation: ${e.message}"
            else -> e.message ?: e.code
        }
        else -> e.message ?: e.javaClass.simpleName
    }

    private fun launchOp(label: String, block: suspend (InstallationApi) -> Unit) {
        val api = api ?: return notice(NoticeKind.ERROR, "Not connected")
        viewModelScope.launch {
            try { block(api) } catch (e: Exception) { notice(NoticeKind.ERROR, "$label failed: ${describe(e)}") }
        }
    }

    // ---- connection ---------------------------------------------------

    fun startScan() {
        scanJob?.cancel()
        _state.update { it.copy(scanning = true, devices = emptyList()) }
        scanJob = viewModelScope.launch {
            try {
                scanForInstallation(adapter).collect { device ->
                    _state.update { s -> if (s.devices.any { it.address == device.address }) s else s.copy(devices = s.devices + device) }
                }
            } catch (e: Exception) {
                notice(NoticeKind.ERROR, "Scan failed: ${describe(e)}")
            } finally {
                _state.update { it.copy(scanning = false) }
            }
        }
    }

    fun stopScan() { scanJob?.cancel() }

    fun connect(device: BluetoothDevice) {
        stopScan()
        disconnect()
        val link = GattTransport(getApplication(), device)
        transport = link
        viewModelScope.launch {
            _state.update { it.copy(link = LinkState.CONNECTING) }
            try {
                link.connect()
                val client = RpcClient(link, viewModelScope)
                rpc = client
                api = InstallationApi(client)
                watchLink(link, client)
                refreshAll()
            } catch (e: Exception) {
                _state.update { it.copy(link = LinkState.DISCONNECTED) }
                notice(NoticeKind.ERROR, "Could not connect: ${describe(e)}")
            }
        }
    }

    private fun watchLink(link: GattTransport, client: RpcClient) {
        linkJobs += viewModelScope.launch { link.state.collect { s -> _state.update { it.copy(link = s) } } }
        // The Pi hints which domains changed; refetch only those.
        linkJobs += viewModelScope.launch {
            client.changed.collect { change ->
                val api = api ?: return@collect
                runCatching {
                    if (change.has(Domain.STATUS)) refreshStatus(api)
                    if (change.has(Domain.LIBRARY)) refreshLibrary(api)
                    if (change.has(Domain.PRINTER)) { refreshJobs(api); refreshStatus(api) }
                    if (change.has(Domain.SCHEDULER)) refreshSchedule(api)
                }
            }
        }
    }

    fun disconnect() {
        linkJobs.forEach { it.cancel() }
        linkJobs.clear()
        rpc?.close()
        transport?.disconnect()
        transport = null; rpc = null; api = null
        thumbnailsRequested.clear()
        _state.update { it.copy(link = LinkState.DISCONNECTED) }
    }

    override fun onCleared() { disconnect() }

    // ---- state refresh --------------------------------------------------

    private suspend fun refreshStatus(api: InstallationApi) { val s = api.status(); _state.update { it.copy(status = s) } }
    private suspend fun refreshLibrary(api: InstallationApi) { val l = api.library(); _state.update { it.copy(library = l) } }
    /** Never throws: a printer that is off must not abort the refresh of everything else. */
    private suspend fun refreshPrinterSettings(api: InstallationApi) {
        try {
            val s = api.printerSettings()
            _state.update { it.copy(printerValues = s.values, printerSettingsError = s.error) }
        } catch (e: Exception) {
            _state.update { it.copy(printerValues = emptyMap(), printerSettingsError = describe(e)) }
        }
    }

    private suspend fun refreshJobs(api: InstallationApi) { val j = api.jobs(); _state.update { it.copy(jobs = j) } }
    private suspend fun refreshSchedule(api: InstallationApi) {
        val s = api.schedule(); val p = api.preview(3)
        _state.update { it.copy(schedule = s, preview = p) }
    }

    fun refreshAll() = launchOp("Refresh") { api ->
        refreshStatus(api); refreshLibrary(api); refreshJobs(api); refreshSchedule(api)
        val history = api.history(50); val stats = api.stats(); val caps = api.capabilities()
        _state.update { it.copy(history = history, stats = stats, capabilities = caps) }
        refreshPrinterSettings(api)
    }

    fun refreshPrinting() = launchOp("Refresh") { api ->
        refreshJobs(api); refreshStatus(api)
        refreshPrinterSettings(api)
        val history = api.history(50); val stats = api.stats()
        _state.update { it.copy(history = history, stats = stats) }
    }

    // ---- commands -----------------------------------------------------------

    fun syncClock() = launchOp("Clock sync") { api ->
        val piNow = api.syncClock()
        refreshStatus(api)
        notice(NoticeKind.COMPLETED, "Pi clock set to ${java.time.Instant.ofEpochMilli(piNow)}")
    }

    fun delete(item: LibraryItem) = launchOp("Delete") { api ->
        api.delete(item.id); refreshLibrary(api)
        notice(NoticeKind.COMPLETED, "Removed ${item.originalFilename}")
    }

    fun print(item: LibraryItem, copies: Int) = launchOp("Print") { api ->
        val job = api.printItem(item.id, copies)
        refreshJobs(api)
        notice(NoticeKind.ACCEPTED, "Accepted: ${job.total} print(s) of ${item.originalFilename} queued. The Pi will print them even if you disconnect.")
    }

    fun printAll(copies: Int) = launchOp("Print all") { api ->
        val job = api.printAll(copies)
        refreshJobs(api)
        notice(NoticeKind.ACCEPTED, "Accepted: print-all queued (${job.total} prints). The Pi owns this job; you can disconnect.")
    }

    fun cancel(job: Job) = launchOp("Cancel") { api ->
        api.cancel(job.id); refreshJobs(api)
        notice(NoticeKind.COMPLETED, "Job cancelled")
    }

    fun setAutoprint(enabled: Boolean) = launchOp("Autoprint") { api ->
        val s = api.setSchedule(enabled = enabled)
        _state.update { it.copy(schedule = s) }
        refreshSchedule(api)
        notice(NoticeKind.COMPLETED, if (enabled) "Autoprint enabled" else "Autoprint disabled")
    }

    fun setSchedule(minMinutes: Double, maxMinutes: Double, ordering: String) = launchOp("Schedule") { api ->
        api.setSchedule(minMinutes = minMinutes, maxMinutes = maxMinutes, ordering = ordering)
        refreshSchedule(api)
        notice(NoticeKind.COMPLETED, "Schedule saved")
    }

    fun setPrinterSetting(key: String, value: Long) = launchOp("Printer setting") { api ->
        api.setPrinterSetting(key, value)
        refreshPrinterSettings(api)
        notice(NoticeKind.COMPLETED, "Printer setting applied")
    }

    /** The printer prints its own settings label (uses a little paper). */
    fun printConfig() = launchOp("Print settings") { api ->
        api.printConfig()
        notice(NoticeKind.COMPLETED, "The printer is printing its settings label")
    }

    /** Sets the daily window autoprint may run in; times are minutes since midnight, Pi local time. */
    fun setPrintWindow(enabled: Boolean, startMinute: Int, endMinute: Int) = launchOp("Autoprint hours") { api ->
        api.setSchedule(windowEnabled = enabled, windowStart = startMinute, windowEnd = endMinute)
        refreshSchedule(api)
        notice(NoticeKind.COMPLETED, if (enabled) "Autoprint limited to the chosen hours" else "Autoprint may run at any hour")
    }

    /** Fetches one item's thumbnail once; failures are silent and retried the next time the item is shown. */
    fun loadThumbnail(id: String) {
        val api = api ?: return
        if (id in _state.value.thumbnails || !thumbnailsRequested.add(id)) return
        viewModelScope.launch {
            try {
                val bytes = api.thumbnail(id)
                val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() ?: return@launch
                _state.update { it.copy(thumbnails = it.thumbnails + (id to bitmap)) }
            } catch (e: Exception) {
                thumbnailsRequested.remove(id)
            }
        }
    }

    fun upload(uri: Uri) {
        val client = rpc ?: return notice(NoticeKind.ERROR, "Not connected")
        viewModelScope.launch {
            val resolver = getApplication<Application>().contentResolver
            try {
                val name = uri.lastPathSegment?.substringAfterLast('/') ?: "artwork"
                val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: error("could not read the file")
                _state.update { it.copy(uploadProgress = 0f) }
                Uploader(client).upload(name, bytes) { sent, total -> _state.update { s -> s.copy(uploadProgress = sent.toFloat() / total) } }
                notice(NoticeKind.ACCEPTED, "Accepted: $name was handed to the Pi for processing. It will appear in the library shortly.")
                // The watcher converts within seconds; look a few times.
                api?.let { api -> repeat(5) { delay(2_000); runCatching { refreshLibrary(api) } } }
            } catch (e: Exception) {
                notice(NoticeKind.ERROR, "Upload failed: ${describe(e)}")
            } finally {
                _state.update { it.copy(uploadProgress = null) }
            }
        }
    }
}
