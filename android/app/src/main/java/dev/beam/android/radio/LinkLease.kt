package dev.beam.android.radio

/** Owns a link even when acquisition finishes after its caller cancels. */
internal class LinkLease {
    private var release: (() -> Unit)? = null
    private var closed = false

    fun attach(action: () -> Unit): Boolean {
        val attached = synchronized(this) {
            if (closed) false else {
                check(release == null) { "A link is already attached" }
                release = action
                true
            }
        }
        if (!attached) action()
        return attached
    }

    fun close() {
        val action = synchronized(this) {
            if (closed) return
            closed = true
            release.also { release = null }
        }
        action?.invoke()
    }
}
