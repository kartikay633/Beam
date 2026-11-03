package dev.beam.android.ui

import dev.beam.android.data.offersRoot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FolderLabelTest {
    @Test
    fun foldersAreNamedTheWayPeopleSayThem() {
        assertEquals("Beam folder", folderLabel(null))
        assertEquals("Desktop", folderLabel("desktop"))
        assertEquals("Trip photos", folderLabel("pin:Trip photos"))
        assertEquals("Pinned folder", folderLabel("pin:"))
        assertEquals("Documents › Projects", folderLabel("documents/Projects"))
        assertEquals("Photos › 2026", folderLabel("pin:Photos/2026"))
    }

    @Test
    fun subfoldersSitUnderTheirFolder() {
        val tree = saveFolderTree(listOf("desktop", "documents", "documents/Projects", "documents/Bills", "pin:Photos"))
        assertEquals(
            listOf(
                "desktop" to emptyList(),
                "documents" to listOf("documents/Projects", "documents/Bills"),
                "pin:Photos" to emptyList(),
            ),
            tree,
        )
    }

    @Test
    fun newFolderNamesWindowsWouldRefuseAreCaught() {
        assertNull(folderNameProblem("Trip 2026"))
        assertEquals("Type a name", folderNameProblem("   "))
        assertEquals("Can't start or end with a dot", folderNameProblem(".."))
        assertTrue(folderNameProblem("a/b")!!.startsWith("Can't contain"))
        assertTrue(folderNameProblem("a:b")!!.startsWith("Can't contain"))
    }

    @Test
    fun aNewSubfolderCountsAsOfferedWhileItsFolderIs() {
        assertTrue(offersRoot(listOf("documents"), "documents/Not made yet"))
        assertFalse(offersRoot(listOf("desktop"), "documents/Projects"))
    }
}
