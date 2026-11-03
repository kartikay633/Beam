package dev.beam.android.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class FolderLabelTest {
    @Test
    fun foldersAreNamedTheWayPeopleSayThem() {
        assertEquals("Beam folder", folderLabel(null))
        assertEquals("Desktop", folderLabel("desktop"))
        assertEquals("Trip photos", folderLabel("pin:Trip photos"))
        assertEquals("Pinned folder", folderLabel("pin:"))
    }
}
