package dev.beam.android.model

import org.junit.Assert.assertEquals
import org.junit.Test

class FormatTest {
    @Test
    fun bytesUseDecimalUnits() {
        assertEquals("999 B", Format.bytes(999))
        assertEquals("1.5 KB", Format.bytes(1_500))
        assertEquals("200 MB", Format.bytes(200_000_000))
        assertEquals("2.3 GB", Format.bytes(2_300_000_000))
    }

    @Test
    fun durationsAreRounded() {
        assertEquals("1 s", Format.duration(0.2))
        assertEquals("45 s", Format.duration(45.0))
        assertEquals("2 min 5 s", Format.duration(125.0))
        assertEquals("1 h 1 min", Format.duration(3_700.0))
    }

    @Test
    fun codesAreSplitForReading() {
        assertEquals("482 193", Format.code("482193"))
    }
}
