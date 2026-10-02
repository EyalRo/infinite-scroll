package art.infinitescroll.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class LibraryItem(
    val id: String,
    @SerialName("original_filename") val originalFilename: String = "",
    @SerialName("added_at") val addedAt: Double = 0.0,
    @SerialName("print_count") val printCount: Int = 0,
    @SerialName("last_printed_at") val lastPrintedAt: Double? = null,
)

@Serializable
data class LibraryPage(
    val total: Int,
    val offset: Int,
    val items: List<LibraryItem>,
    @SerialName("next_offset") val nextOffset: Int? = null,
)

@Serializable
data class Job(
    val id: String,
    val kind: String,
    val items: List<String> = emptyList(),
    val copies: Int = 1,
    val done: Int = 0,
    val skipped: Int = 0,
    val state: String,
    @SerialName("created_at") val createdAt: Double = 0.0,
    @SerialName("finished_at") val finishedAt: Double? = null,
    val error: String? = null,
) {
    val total: Int get() = items.size * copies
    val active: Boolean get() = state == "queued" || state == "running"
}

@Serializable
data class JobList(val jobs: List<Job> = emptyList())

@Serializable
data class JobAck(val job: Job)

@Serializable
data class Schedule(
    val enabled: Boolean = false,
    @SerialName("min_minutes") val minMinutes: Double = 15.0,
    @SerialName("max_minutes") val maxMinutes: Double = 20.0,
    val ordering: String = "sequential",
    @SerialName("next_print_at") val nextPrintAt: Double? = null,
    @SerialName("last_item_id") val lastItemId: String? = null,
    @SerialName("last_error") val lastError: String? = null,
    /** Autoprint runs only inside this daily window, in Pi local time (minutes since midnight; `[start, end)`, wraps midnight if start > end). */
    @SerialName("window_enabled") val windowEnabled: Boolean = true,
    @SerialName("window_start") val windowStart: Int = 600,
    @SerialName("window_end") val windowEnd: Int = 960,
)

@Serializable
data class Preview(val ordering: String, val exact: Boolean, val picks: List<LibraryItem> = emptyList())

@Serializable
data class PrinterState(val connected: Boolean = false, val busy: Boolean = false)

@Serializable
data class JobsSummary(val active: Int = 0, val current: Job? = null)

/** The printer service's own `/status`, passed through by `sys.status`. */
@Serializable
data class PrinterService(
    val printer: PrinterState = PrinterState(),
    val error: String? = null,
    @SerialName("catalog_count") val catalogCount: Int = 0,
    @SerialName("failed_count") val failedCount: Int = 0,
    @SerialName("processing_count") val processingCount: Int? = null,
    val jobs: JobsSummary = JobsSummary(),
    val autoprint: Schedule = Schedule(),
)

@Serializable
data class Clock(@SerialName("unix_ms") val unixMs: Long)

@Serializable
data class Services(val printer: Boolean = false, val uploader: Boolean = false)

@Serializable
data class Status(
    val proto: Int,
    val clock: Clock,
    val services: Services = Services(),
    val printer: PrinterService? = null,
    @SerialName("printer_error") val printerError: String? = null,
)

@Serializable
data class HistoryRecord(
    val at: Double,
    val origin: String,
    @SerialName("item_id") val itemId: String,
    @SerialName("original_filename") val originalFilename: String = "",
    @SerialName("job_id") val jobId: String? = null,
    val success: Boolean,
    val error: String? = null,
    @SerialName("paper_mm") val paperMm: Double? = null,
)

@Serializable
data class History(val recent: List<HistoryRecord> = emptyList())

@Serializable
data class Stats(
    @SerialName("library_items") val libraryItems: Int = 0,
    @SerialName("failed_conversions") val failedConversions: Int = 0,
    @SerialName("counters_since") val countersSince: Double? = null,
    @SerialName("prints_ok") val printsOk: Long = 0,
    @SerialName("prints_failed") val printsFailed: Long = 0,
    @SerialName("scheduled_ok") val scheduledOk: Long = 0,
    @SerialName("job_ok") val jobOk: Long = 0,
    @SerialName("paper_mm") val paperMm: Double = 0.0,
)

@Serializable
data class Capability(
    val key: String,
    val label: String,
    val min: Long,
    val max: Long,
    val step: Long = 1,
    val readable: Boolean = true,
    val writable: Boolean = false,
)

/** A one-shot command the printer offers, e.g. printing its own settings label. */
@Serializable
data class PrinterAction(val key: String, val label: String)

@Serializable
data class Capabilities(val schema: Int = 1, val settings: List<Capability> = emptyList(), val actions: List<PrinterAction> = emptyList())

/** Live values read from the printer; `error` is set (with empty `values`) when it could not be reached. */
@Serializable
data class PrinterSettings(val schema: Int = 1, val values: Map<String, Long> = emptyMap(), val error: String? = null)

/** A small JPEG of one library item, base64 in `data`. */
@Serializable
data class Thumbnail(val id: String, val format: String = "jpeg", val width: Int = 0, val height: Int = 0, val data: String)

@Serializable
data class UploadAck(@SerialName("item_id") val itemId: String? = null, val filename: String? = null, val name: String = "")

/** One nearby Wi-Fi network; `signal` is 0-100 and `security` is `open`, `wpa` or `enterprise` (unsupported). */
@Serializable
data class WifiNetwork(
    val ssid: String,
    val signal: Int = 0,
    val security: String = "wpa",
    @SerialName("in_use") val inUse: Boolean = false,
)

@Serializable
data class WifiScan(val networks: List<WifiNetwork> = emptyList())

/** The Pi's own record of the last join attempt: `idle`, `connecting`, `connected` or `failed`. */
@Serializable
data class WifiAttempt(val state: String = "idle", val ssid: String? = null, val error: String? = null)

@Serializable
data class WifiStatus(
    val connected: Boolean = false,
    val ssid: String? = null,
    val signal: Int = 0,
    val security: String? = null,
    val attempt: WifiAttempt = WifiAttempt(),
)

/** Signal strength as a 0-4 bar count; the UI shows bars, never the number. */
fun wifiBars(signal: Int): Int = when {
    signal <= 0 -> 0
    signal < 25 -> 1
    signal < 50 -> 2
    signal < 75 -> 3
    else -> 4
}
