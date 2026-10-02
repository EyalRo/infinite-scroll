package art.infinitescroll.protocol

/** Framing for the Request/Response characteristics. See docs/ble-protocol.md. */
object Frames {
    const val HEADER_LEN = 3
    const val FLAG_FIRST = 0b01
    const val FLAG_LAST = 0b10
    const val MAX_MESSAGE_BYTES = 64 * 1024

    fun encode(msgId: Int, body: ByteArray, chunkPayload: Int): List<ByteArray> {
        val size = maxOf(1, chunkPayload)
        val frames = ArrayList<ByteArray>()
        var offset = 0
        while (true) {
            val end = minOf(offset + size, body.size)
            var flags = 0
            if (offset == 0) flags = flags or FLAG_FIRST
            if (end == body.size) flags = flags or FLAG_LAST
            val frame = ByteArray(HEADER_LEN + end - offset)
            frame[0] = flags.toByte()
            frame[1] = (msgId and 0xFF).toByte()
            frame[2] = ((msgId shr 8) and 0xFF).toByte()
            body.copyInto(frame, HEADER_LEN, offset, end)
            frames += frame
            if (end == body.size) return frames
            offset = end
        }
    }
}

class FrameException(message: String) : Exception(message)

/** Reassembles frames into whole messages; a FIRST frame always restarts. */
class Reassembler {
    private var msgId: Int? = null
    private var buffer = java.io.ByteArrayOutputStream()

    /** Returns (messageId, body) when [frame] completes a message, else null. */
    fun push(frame: ByteArray): Pair<Int, ByteArray>? {
        if (frame.size < Frames.HEADER_LEN) throw FrameException("frame too short")
        val flags = frame[0].toInt() and 0xFF
        val id = (frame[1].toInt() and 0xFF) or ((frame[2].toInt() and 0xFF) shl 8)
        if (flags and Frames.FLAG_FIRST != 0) {
            buffer = java.io.ByteArrayOutputStream()
            msgId = id
        } else if (msgId != id) {
            reset()
            throw FrameException("unexpected continuation frame")
        }
        if (buffer.size() + frame.size - Frames.HEADER_LEN > Frames.MAX_MESSAGE_BYTES) {
            reset()
            throw FrameException("message too large")
        }
        buffer.write(frame, Frames.HEADER_LEN, frame.size - Frames.HEADER_LEN)
        if (flags and Frames.FLAG_LAST != 0) {
            val body = buffer.toByteArray()
            reset()
            return id to body
        }
        return null
    }

    private fun reset() {
        msgId = null
        buffer = java.io.ByteArrayOutputStream()
    }
}
