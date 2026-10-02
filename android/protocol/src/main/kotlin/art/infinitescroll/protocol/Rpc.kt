package art.infinitescroll.protocol

import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job as CoroutineJob
import kotlinx.coroutines.async
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** The BLE link as the protocol sees it; implemented over GATT in :app. */
interface Transport {
    /** Negotiated ATT MTU (23 until negotiated). */
    val mtu: Int
    val responseFrames: Flow<ByteArray>
    val changed: Flow<Changed>
    suspend fun writeRequestFrame(frame: ByteArray)
    suspend fun writeUploadChunk(chunk: ByteArray)
}

/** One request in flight at a time, matched to its response by id. */
class RpcClient(private val transport: Transport, scope: CoroutineScope, private val timeoutMs: Long = 15_000) {
    private val messages = MutableSharedFlow<Pair<Int, Reply>>(extraBufferCapacity = 64)
    private val lock = Mutex()
    private var nextId = 1
    // UNDISPATCHED: subscribe to the response stream before any request can
    // be sent; a hot flow drops frames that arrive before the collector exists.
    private val collector: CoroutineJob = scope.launch(start = CoroutineStart.UNDISPATCHED) {
        val reassembler = Reassembler()
        transport.responseFrames.collect { frame ->
            try {
                reassembler.push(frame)?.let { (_, body) -> messages.emit(parseReply(body)) }
            } catch (_: FrameException) {
                // A corrupt response simply times out the caller; the next FIRST frame resyncs.
            } catch (_: Exception) {
            }
        }
    }

    val changed: Flow<Changed> get() = transport.changed
    val mtu: Int get() = transport.mtu

    fun close() = collector.cancel()

    suspend fun call(op: String, args: JsonObject = JsonObject(emptyMap())): Reply = lock.withLock {
        val id = nextId
        nextId = if (nextId >= 0xFFFF) 1 else nextId + 1
        val chunkPayload = transport.mtu - Wire.ATT_OVERHEAD - Frames.HEADER_LEN
        withTimeout(timeoutMs) {
            kotlinx.coroutines.coroutineScope {
                // Subscribe before writing so a fast response is never missed.
                val reply = async(start = CoroutineStart.UNDISPATCHED) { messages.first { it.first == id }.second }
                for (frame in Frames.encode(id, buildRequest(id, op, args), chunkPayload)) transport.writeRequestFrame(frame)
                reply.await()
            }
        }
    }

    /** Like [call] but throws [RpcException] for an error reply. */
    suspend fun request(op: String, args: JsonObject = JsonObject(emptyMap())): Reply.Ok =
        when (val reply = call(op, args)) {
            is Reply.Ok -> reply
            is Reply.Err -> throw RpcException(reply.code, reply.message, reply.details)
        }

    suspend fun writeUpload(chunk: ByteArray) = transport.writeUploadChunk(chunk)
}

/** Result of a finished upload commit: the Pi has accepted it for conversion. */
data class UploadResult(val itemId: String?, val filename: String?)

/**
 * Streams a file over the Upload characteristic, resuming from the Pi's
 * reported offset if chunks were lost, then commits. Success means ACCEPTED:
 * the Pi has handed the image to its normal processing pipeline.
 */
class Uploader(private val rpc: RpcClient, private val maxRetries: Int = 3) {
    suspend fun upload(name: String, bytes: ByteArray, onProgress: (Int, Int) -> Unit = { _, _ -> }): UploadResult {
        require(bytes.isNotEmpty()) { "empty file" }
        val begin = rpc.request("upload.begin", jsonArgs("name" to name, "size" to bytes.size)).result
        val session = (begin["upload_id"] as JsonPrimitive).content.toInt()
        val chunkData = rpc.mtu - Wire.ATT_OVERHEAD - Wire.UPLOAD_HEADER_LEN
        require(chunkData > 0) { "MTU too small for uploads" }

        var from = 0
        var attempts = 0
        while (true) {
            while (from < bytes.size) {
                val to = min(from + chunkData, bytes.size)
                rpc.writeUpload(Wire.uploadChunk(session, from.toLong(), bytes, from, to))
                from = to
                onProgress(from, bytes.size)
            }
            val status = rpc.request("upload.status", jsonArgs("upload_id" to session)).result
            val received = (status["received"] as JsonPrimitive).content.toInt()
            if (received >= bytes.size) break
            if (++attempts > maxRetries) {
                runCatching { rpc.request("upload.abort", jsonArgs("upload_id" to session)) }
                throw RpcException("upload_incomplete", "the Pi received only $received of ${bytes.size} bytes")
            }
            from = received // resend from what the Pi actually holds
        }
        val ack = rpc.request("upload.commit", jsonArgs("upload_id" to session, "crc32" to Wire.crc32(bytes)))
        val parsed = Wire.json.decodeFromJsonElement(UploadAck.serializer(), ack.result)
        return UploadResult(parsed.itemId, parsed.filename)
    }
}

/** Typed wrappers over the protocol's operations. Holds no state. */
class InstallationApi(val rpc: RpcClient) {
    private inline fun <reified T> decode(ok: Reply.Ok, serializer: kotlinx.serialization.KSerializer<T>): T =
        Wire.json.decodeFromJsonElement(serializer, ok.result)

    suspend fun status(): Status = decode(rpc.request("sys.status"), Status.serializer())

    suspend fun libraryPage(offset: Int = 0, limit: Int = 50): LibraryPage =
        decode(rpc.request("library.list", jsonArgs("offset" to offset, "limit" to limit)), LibraryPage.serializer())

    suspend fun library(): List<LibraryItem> {
        val all = ArrayList<LibraryItem>()
        var offset: Int? = 0
        while (offset != null) {
            val page = libraryPage(offset)
            all += page.items
            offset = page.nextOffset
        }
        return all
    }

    suspend fun delete(id: String) { rpc.request("library.delete", jsonArgs("id" to id)) }

    /** Accepted, not finished: the returned job is owned by the Pi. */
    suspend fun printItem(id: String, copies: Int = 1): Job =
        decode(rpc.request("print.item", jsonArgs("id" to id, "copies" to copies)), JobAck.serializer()).job

    suspend fun printAll(copies: Int = 1): Job =
        decode(rpc.request("print.all", jsonArgs("copies" to copies)), JobAck.serializer()).job

    suspend fun jobs(): List<Job> = decode(rpc.request("print.jobs"), JobList.serializer()).jobs
    suspend fun cancel(jobId: String) { rpc.request("print.cancel", jsonArgs("job_id" to jobId)) }
    suspend fun history(limit: Int = 50): List<HistoryRecord> =
        decode(rpc.request("print.history", jsonArgs("limit" to limit)), History.serializer()).recent

    suspend fun stats(): Stats = decode(rpc.request("stats.get"), Stats.serializer())
    suspend fun schedule(): Schedule = decode(rpc.request("sched.get"), Schedule.serializer())

    suspend fun setSchedule(enabled: Boolean? = null, minMinutes: Double? = null, maxMinutes: Double? = null, ordering: String? = null): Schedule =
        decode(
            rpc.request("sched.set", jsonArgs("enabled" to enabled, "min_minutes" to minMinutes, "max_minutes" to maxMinutes, "ordering" to ordering)),
            Schedule.serializer(),
        )

    suspend fun preview(count: Int = 3): Preview = decode(rpc.request("sched.preview", jsonArgs("count" to count)), Preview.serializer())
    suspend fun capabilities(): Capabilities = decode(rpc.request("printer.capabilities"), Capabilities.serializer())
    suspend fun setPrinterSetting(key: String, value: Long) { rpc.request("printer.settings.set", jsonArgs("key" to key, "value" to value)) }

    /** Sets the Pi clock from the phone's clock; returns the Pi's resulting time. */
    suspend fun syncClock(nowMs: Long = System.currentTimeMillis()): Long =
        (rpc.request("sys.clock.set", jsonArgs("unix_ms" to nowMs)).result["unix_ms"] as JsonPrimitive).content.toLong()

    suspend fun clock(): Long = (rpc.request("sys.clock.get").result["unix_ms"] as JsonPrimitive).content.toLong()
}
