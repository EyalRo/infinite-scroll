package art.infinitescroll.protocol

import java.util.UUID
import java.util.zip.CRC32
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

object Wire {
    const val PROTOCOL_VERSION = 1

    val SERVICE: UUID = UUID.fromString("a4f60001-6b8e-4c1f-9d0a-5e1f0c1a1001")
    val INFO: UUID = UUID.fromString("a4f60002-6b8e-4c1f-9d0a-5e1f0c1a1001")
    val REQUEST: UUID = UUID.fromString("a4f60003-6b8e-4c1f-9d0a-5e1f0c1a1001")
    val RESPONSE: UUID = UUID.fromString("a4f60004-6b8e-4c1f-9d0a-5e1f0c1a1001")
    val CHANGED: UUID = UUID.fromString("a4f60005-6b8e-4c1f-9d0a-5e1f0c1a1001")
    val UPLOAD: UUID = UUID.fromString("a4f60006-6b8e-4c1f-9d0a-5e1f0c1a1001")

    const val UPLOAD_HEADER_LEN = 6
    const val ATT_OVERHEAD = 3

    /** Android's GATT stack drops characteristic values over 512 bytes, whatever the MTU. */
    const val MAX_ATTR_VALUE = 512

    /** Largest characteristic value usable at this MTU. */
    fun maxValue(mtu: Int): Int = minOf(mtu - ATT_OVERHEAD, MAX_ATTR_VALUE)

    val json = Json { ignoreUnknownKeys = true }

    fun crc32(bytes: ByteArray): Long = CRC32().apply { update(bytes) }.value

    /** Upload characteristic payload: session u16 LE | offset u32 LE | data. */
    fun uploadChunk(session: Int, offset: Long, data: ByteArray, from: Int, to: Int): ByteArray {
        val out = ByteArray(UPLOAD_HEADER_LEN + to - from)
        out[0] = (session and 0xFF).toByte()
        out[1] = ((session shr 8) and 0xFF).toByte()
        for (i in 0 until 4) out[2 + i] = ((offset shr (8 * i)) and 0xFF).toByte()
        data.copyInto(out, UPLOAD_HEADER_LEN, from, to)
        return out
    }
}

/** Change-notification domains (bits of the Changed characteristic). */
enum class Domain(val bit: Int) { STATUS(1), LIBRARY(2), PRINTER(4), SCHEDULER(8) }

data class Changed(val mask: Int, val revision: Long) {
    fun has(domain: Domain) = mask and domain.bit != 0

    companion object {
        fun parse(bytes: ByteArray): Changed? {
            if (bytes.size < 6) return null
            val mask = (bytes[0].toInt() and 0xFF) or ((bytes[1].toInt() and 0xFF) shl 8)
            var revision = 0L
            for (i in 0 until 4) revision = revision or ((bytes[2 + i].toLong() and 0xFF) shl (8 * i))
            return Changed(mask, revision)
        }
    }
}

/**
 * Whether the Pi finished an operation or only took ownership of it. An
 * ACCEPTED operation continues on the Pi even if Bluetooth disconnects.
 */
enum class Disposition { COMPLETED, ACCEPTED }

sealed interface Reply {
    data class Ok(val disposition: Disposition, val result: JsonObject) : Reply
    data class Err(val code: String, val message: String, val details: JsonObject?) : Reply
}

class RpcException(val code: String, message: String, val details: JsonObject? = null) : Exception(message)

fun buildRequest(id: Int, op: String, args: JsonObject = JsonObject(emptyMap())): ByteArray =
    buildJsonObject {
        put("v", Wire.PROTOCOL_VERSION)
        put("id", id)
        put("op", op)
        put("args", args)
    }.toString().toByteArray(Charsets.UTF_8)

fun parseReply(body: ByteArray): Pair<Int, Reply> {
    val root = Wire.json.parseToJsonElement(body.toString(Charsets.UTF_8)).jsonObject
    val id = root["id"]?.jsonPrimitive?.intOrNull ?: 0
    if (root["ok"]?.jsonPrimitive?.booleanOrNull == true) {
        val disposition = if (root["disposition"]?.jsonPrimitive?.contentOrNull == "accepted") Disposition.ACCEPTED else Disposition.COMPLETED
        return id to Reply.Ok(disposition, (root["result"] as? JsonObject) ?: JsonObject(emptyMap()))
    }
    val error = root["error"]?.jsonObject
    return id to Reply.Err(
        error?.get("code")?.jsonPrimitive?.contentOrNull ?: "internal",
        error?.get("message")?.jsonPrimitive?.contentOrNull ?: "request failed",
        error?.get("details") as? JsonObject,
    )
}

fun jsonArgs(vararg pairs: Pair<String, Any?>): JsonObject = buildJsonObject {
    for ((key, value) in pairs) {
        when (value) {
            null -> Unit
            is Boolean -> put(key, value)
            is Int -> put(key, value)
            is Long -> put(key, value)
            is Double -> put(key, value)
            is String -> put(key, value)
            is JsonElement -> put(key, value)
            else -> put(key, JsonPrimitive(value.toString()))
        }
    }
}
