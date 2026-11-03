package dev.beam.android.radio

/** A P2P channel does not own a group: old cleanup must not remove a newer one. */
internal class P2pGeneration {
    private var generation = 0L

    @Synchronized fun begin(): Long = ++generation

    @Synchronized fun release(token: Long, current: () -> Unit, obsolete: () -> Unit) {
        if (token == generation) current() else obsolete()
    }
}
