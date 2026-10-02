package art.infinitescroll.protocol

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

class WireVectorsTest {
    // These vectors are mirrored in crates/bluetooth/src/protocol.rs tests;
    // both sides must agree byte for byte.
    @Test fun singleFrameBytes() {
        val frames = Frames.encode(7, "hello".toByteArray(), 100)
        assertEquals(1, frames.size)
        assertContentEquals(byteArrayOf(0x03, 0x07, 0x00, 'h'.code.toByte(), 'e'.code.toByte(), 'l'.code.toByte(), 'l'.code.toByte(), 'o'.code.toByte()), frames[0])
    }

    @Test fun uploadChunkBytes() {
        val chunk = Wire.uploadChunk(2, 16, byteArrayOf(9, 8, 7), 0, 3)
        assertContentEquals(byteArrayOf(2, 0, 16, 0, 0, 0, 9, 8, 7), chunk)
    }

    @Test fun crc32MatchesTheStandardCheckValue() {
        assertEquals(0xCBF43926L, Wire.crc32("123456789".toByteArray()))
    }

    @Test fun changedParses() {
        val changed = assertNotNull(Changed.parse(byteArrayOf(0x0A, 0x00, 0x05, 0x00, 0x00, 0x00)))
        assertTrue(changed.has(Domain.LIBRARY) && changed.has(Domain.SCHEDULER))
        assertTrue(!changed.has(Domain.STATUS))
        assertEquals(5L, changed.revision)
        assertNull(Changed.parse(byteArrayOf(1, 2)))
    }

    @Test fun multiFrameRoundTripAndResync() {
        val body = ByteArray(1000) { (it % 251).toByte() }
        val frames = Frames.encode(0xBEEF, body, 17)
        val r = Reassembler()
        var out: Pair<Int, ByteArray>? = null
        for (f in frames) out = r.push(f)
        assertEquals(0xBEEF, out!!.first)
        assertContentEquals(body, out.second)

        // An abandoned message must not poison the next one.
        r.push(Frames.encode(1, ByteArray(40), 10)[0])
        val fresh = r.push(Frames.encode(2, "ok".toByteArray(), 10)[0])
        assertEquals(2, fresh!!.first)
        assertFailsWith<FrameException> { r.push(Frames.encode(3, ByteArray(40), 10)[1]) }
    }

    @Test fun repliesDistinguishAcceptedFromCompleted() {
        val accepted = parseReply("""{"v":1,"id":4,"ok":true,"disposition":"accepted","result":{"job":{"id":"j"}}}""".toByteArray())
        assertEquals(Disposition.ACCEPTED, (accepted.second as Reply.Ok).disposition)
        val completed = parseReply("""{"v":1,"id":5,"ok":true,"disposition":"completed","result":{}}""".toByteArray())
        assertEquals(Disposition.COMPLETED, (completed.second as Reply.Ok).disposition)
        val error = parseReply("""{"v":1,"id":6,"ok":false,"error":{"code":"not_found","message":"nope"}}""".toByteArray())
        assertEquals("not_found", (error.second as Reply.Err).code)
    }

    @Test fun modelsIgnoreUnknownFieldsAndParsePiShapes() {
        val job = Wire.json.decodeFromString(Job.serializer(), """{"id":"j","kind":"all","items":["a","b"],"copies":2,"done":1,"skipped":0,"state":"running","created_at":1.0,"finished_at":null,"error":null,"consecutive_failures":0,"future":1}""")
        assertEquals(4, job.total)
        assertTrue(job.active)
        val status = Wire.json.decodeFromString(Status.serializer(), """{"proto":1,"clock":{"unix_ms":1800000000000},"services":{"printer":true,"uploader":true},"printer":{"printer":{"connected":true,"busy":false},"error":null,"catalog_count":2,"failed_count":0,"processing_count":0,"jobs":{"active":0,"current":null},"autoprint":{"enabled":false,"min_minutes":15.0,"max_minutes":20.0,"ordering":"sequential","next_print_at":null,"last_item_id":null,"last_error":null}},"printer_error":null,"bluetooth":{"version":"0.1.0"}}""")
        assertTrue(status.printer!!.printer.connected)
        assertEquals(15.0, status.printer!!.autoprint.minMinutes)
    }
}

/** A scripted stand-in for the Pi, speaking the framed protocol. */
private class FakePi(override val mtu: Int = 50, private val dropFirstUploadChunk: Boolean = false) : Transport {
    override val responseFrames = MutableSharedFlow<ByteArray>(extraBufferCapacity = 256)
    override val changed = MutableSharedFlow<Changed>(extraBufferCapacity = 8)
    private val reassembler = Reassembler()
    val upload = java.io.ByteArrayOutputStream()
    var committed: ByteArray? = null
    private var dropped = false
    val ops = mutableListOf<String>()

    override suspend fun writeRequestFrame(frame: ByteArray) {
        val (id, body) = reassembler.push(frame) ?: return
        val request = Wire.json.parseToJsonElement(body.toString(Charsets.UTF_8)).jsonObject
        val op = request["op"]!!.jsonPrimitive.content
        val args = request["args"]!!.jsonObject
        ops += op
        val reply: JsonObject = when (op) {
            "upload.begin" -> ok(id, "completed", buildJsonObject { put("upload_id", 1); put("size", args["size"]!!.jsonPrimitive.int); put("received", 0) })
            "upload.status" -> ok(id, "completed", buildJsonObject { put("received", upload.size()) })
            "upload.commit" -> {
                val crc = args["crc32"]!!.jsonPrimitive.content.toLong()
                if (Wire.crc32(upload.toByteArray()) != crc) err(id, "invalid_argument") else {
                    committed = upload.toByteArray()
                    ok(id, "accepted", buildJsonObject { put("item_id", "new"); put("filename", "new.png"); put("name", "x") })
                }
            }
            "print.all" -> ok(id, "accepted", buildJsonObject { put("accepted", true); put("job", buildJsonObject { put("id", "j1"); put("kind", "all"); put("state", "queued") }) })
            else -> err(id, "unknown_op")
        }
        // The Pi chunks its notifications to the MTU, like the real service.
        for (f in Frames.encode(id, reply.toString().toByteArray(), mtu - 6)) responseFrames.emit(f)
    }

    override suspend fun writeUploadChunk(chunk: ByteArray) {
        val offset = (0 until 4).fold(0L) { acc, i -> acc or ((chunk[2 + i].toLong() and 0xFF) shl (8 * i)) }.toInt()
        if (dropFirstUploadChunk && !dropped && offset == 0) { dropped = true; return }
        if (offset == upload.size()) upload.write(chunk, 6, chunk.size - 6) // gap or duplicate: ignored, like the Pi
    }

    private fun ok(id: Int, disposition: String, result: JsonObject) = buildJsonObject {
        put("v", 1); put("id", id); put("ok", true); put("disposition", disposition); put("result", result)
    }

    private fun err(id: Int, code: String) = buildJsonObject {
        put("v", 1); put("id", id); put("ok", false)
        put("error", buildJsonObject { put("code", code); put("message", "m") })
    }
}

class RpcTest {
    private fun client(pi: FakePi) = RpcClient(pi, CoroutineScope(Dispatchers.Default), timeoutMs = 5_000)

    @Test fun printAllIsReportedAsAcceptedNotCompleted() = runBlocking {
        val api = InstallationApi(client(FakePi()))
        val reply = api.rpc.request("print.all", jsonArgs("copies" to 1))
        assertEquals(Disposition.ACCEPTED, reply.disposition)
        assertEquals("queued", api.printAll().state)
    }

    @Test fun errorRepliesBecomeExceptions() = runBlocking {
        val rpc = client(FakePi())
        val e = assertFailsWith<RpcException> { rpc.request("shell.exec") }
        assertEquals("unknown_op", e.code)
    }

    @Test fun uploadStreamsAndCommitsWithMatchingCrc() = runBlocking {
        val pi = FakePi()
        val data = ByteArray(5000) { (it * 7).toByte() }
        val result = Uploader(client(pi)).upload("a.png", data)
        assertEquals("new", result.itemId)
        assertContentEquals(data, pi.committed)
    }

    @Test fun uploadResumesFromWhatThePiReportsAfterALostChunk() = runBlocking {
        val pi = FakePi(dropFirstUploadChunk = true)
        val data = ByteArray(3000) { (it % 200).toByte() }
        Uploader(client(pi)).upload("a.png", data)
        assertContentEquals(data, pi.committed)
        assertTrue(pi.ops.count { it == "upload.status" } >= 2)
    }
}
