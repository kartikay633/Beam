package dev.beam.android.radio

/** Reference-counted ownership of an established link. */
internal class RetainedLink<T>(val value: T, private val dispose: (T) -> Unit) {
    class Lease<T>(val value: T, private val release: () -> Unit) {
        private val released = java.util.concurrent.atomic.AtomicBoolean()
        fun release() { if (released.compareAndSet(false, true)) release.invoke() }
    }

    private var closed = false
    private var users = 0
    private var generation = 0L
    val isOpen: Boolean @Synchronized get() = !closed
    val inUse: Boolean @Synchronized get() = !closed && users > 0

    @Synchronized fun acquire(onIdle: (Long) -> Unit): Lease<T>? {
        if (closed) return null
        users++
        generation++
        return Lease(value) {
            val idle = synchronized(this) {
                users--
                if (!closed && users == 0) ++generation else null
            }
            idle?.let(onIdle)
        }
    }

    @Synchronized fun expire(token: Long) {
        if (users == 0 && token == generation) close()
    }

    @Synchronized fun close() {
        if (!closed) { closed = true; dispose(value) }
    }
}
