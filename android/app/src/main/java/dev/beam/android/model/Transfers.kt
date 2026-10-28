package dev.beam.android.model

/**
 * One row in the transfer list. Plain Kotlin (no generated binding types),
 * so the logic around it is unit-testable on the JVM.
 */
data class TransferUi(
    val id: String,
    val sending: Boolean,
    val peerName: String,
    val fileNames: List<String>,
    val fileCount: Int,
    val totalBytes: Long,
    val doneBytes: Long = 0,
    val phase: Phase = Phase.WAITING,
    /** What it's doing ("Waiting for Laptop to accept…") or why it ended. */
    val status: String? = null,
    val verificationCode: String? = null,
    val bytesPerSecond: Double = 0.0,
    val etaSeconds: Double? = null,
    /** content:// URIs of received files, once saved to Downloads. */
    val savedUris: List<String> = emptyList(),
    internal val sampleAtMs: Long = 0,
    internal val sampleBytes: Long = 0,
) {
    enum class Phase { WAITING, ACTIVE, DONE, DECLINED, CANCELLED, FAILED, INTERRUPTED }

    val finished: Boolean
        get() = phase in setOf(Phase.DONE, Phase.DECLINED, Phase.CANCELLED, Phase.FAILED)

    val fraction: Float
        get() = when {
            phase == Phase.DONE -> 1f
            totalBytes <= 0 -> 0f
            else -> (doneBytes.toDouble() / totalBytes).toFloat().coerceIn(0f, 1f)
        }
}

/** Pure state transitions for the transfer list (newest first). */
object TransferReducer {
    /** Speed samples are taken at most this often. */
    private const val SAMPLE_MS = 500L
    /** Older entries beyond this many finished ones are dropped. */
    const val MAX_FINISHED = 30

    fun started(list: List<TransferUi>, transfer: TransferUi): List<TransferUi> {
        val existing = list.firstOrNull { it.id == transfer.id }
        // A resumed transfer keeps its row and progress.
        val row = existing?.copy(phase = TransferUi.Phase.ACTIVE, status = null) ?: transfer
        return trim(listOf(row) + list.filterNot { it.id == transfer.id })
    }

    fun status(list: List<TransferUi>, id: String, status: String, code: String?): List<TransferUi> =
        list.update(id) { t ->
            val phase = if (status.startsWith("Connection lost")) TransferUi.Phase.INTERRUPTED
            else if (t.phase == TransferUi.Phase.INTERRUPTED) TransferUi.Phase.ACTIVE
            else t.phase
            t.copy(status = status, verificationCode = code ?: t.verificationCode, phase = phase)
        }

    fun progress(list: List<TransferUi>, id: String, done: Long, total: Long, nowMs: Long): List<TransferUi> =
        list.update(id) { t ->
            var rate = t.bytesPerSecond
            var sampleAt = t.sampleAtMs
            var sampleBytes = t.sampleBytes
            if (sampleAt == 0L) {
                sampleAt = nowMs
                sampleBytes = done
            } else if (nowMs - sampleAt >= SAMPLE_MS) {
                val sample = (done - sampleBytes).coerceAtLeast(0) * 1000.0 / (nowMs - sampleAt)
                rate = if (rate == 0.0) sample else 0.6 * rate + 0.4 * sample
                sampleAt = nowMs
                sampleBytes = done
            }
            t.copy(
                phase = TransferUi.Phase.ACTIVE,
                doneBytes = done,
                totalBytes = total,
                bytesPerSecond = rate,
                etaSeconds = if (rate > 0) (total - done).coerceAtLeast(0) / rate else null,
                sampleAtMs = sampleAt,
                sampleBytes = sampleBytes,
            )
        }

    fun finished(list: List<TransferUi>, id: String, phase: TransferUi.Phase, message: String?): List<TransferUi> =
        trim(list.update(id) { t ->
            t.copy(
                phase = phase,
                status = message,
                bytesPerSecond = 0.0,
                etaSeconds = null,
                doneBytes = if (phase == TransferUi.Phase.DONE) t.totalBytes else t.doneBytes,
            )
        })

    fun saved(list: List<TransferUi>, id: String, uris: List<String>): List<TransferUi> =
        list.update(id) { it.copy(savedUris = it.savedUris + uris) }

    fun clearFinished(list: List<TransferUi>): List<TransferUi> = list.filterNot { it.finished }

    private fun trim(list: List<TransferUi>): List<TransferUi> {
        var finished = 0
        return list.filter { !it.finished || ++finished <= MAX_FINISHED }
    }

    private inline fun List<TransferUi>.update(id: String, change: (TransferUi) -> TransferUi) =
        map { if (it.id == id) change(it) else it }
}
