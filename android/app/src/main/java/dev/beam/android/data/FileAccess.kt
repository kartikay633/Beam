package dev.beam.android.data

import android.content.Context
import androidx.core.net.toUri
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import dev.beam.core.FileSource
import dev.beam.core.OutgoingFile
import java.io.File
import java.util.UUID

/** A file picked or shared, waiting for the user to choose a device. */
data class StagedFile(val uri: String, val name: String, val size: Long?)

/** Files opened for sending, and what to delete once the send is over. */
class OpenedFiles(val files: List<OutgoingFile>, val cleanup: () -> Unit)

/** Reads shared files. Faked in tests. */
interface FileAccess {
    fun describe(uri: String): StagedFile?
    fun open(files: List<StagedFile>): OpenedFiles
}

/**
 * Content URIs become file descriptors handed to Rust (DECISIONS.md, "Content
 * URIs"): core reads chunks at any offset from parallel streams, so the
 * descriptor must be a seekable regular file. Anything else — a pipe from a
 * cloud provider, a size the provider can't tell — is copied to the cache
 * first and sent from there.
 */
class AndroidFileAccess(private val context: Context) : FileAccess {
    private val resolver get() = context.contentResolver
    private val outgoingCache = File(context.cacheDir, "outgoing")

    init {
        // Copies left behind by a send the app didn't outlive.
        outgoingCache.deleteRecursively()
    }

    override fun describe(uri: String): StagedFile? {
        val parsed = uri.toUri()
        var name: String? = null
        var size: Long? = null
        runCatching {
            resolver.query(parsed, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    name = c.getString(0)
                    size = if (c.isNull(1)) null else c.getLong(1)
                }
            }
        }
        return StagedFile(uri, name ?: parsed.lastPathSegment?.substringAfterLast('/') ?: "file", size)
    }

    override fun open(files: List<StagedFile>): OpenedFiles {
        val opened = mutableListOf<OutgoingFile>()
        val copies = File(outgoingCache, UUID.randomUUID().toString())
        try {
            for (file in files) opened += openOne(file, copies)
        } catch (e: Exception) {
            // Descriptors not yet handed to Rust are still ours to close.
            opened.forEach { (it.source as? FileSource.Fd)?.let { fd -> ParcelFileDescriptor.adoptFd(fd.fd).close() } }
            copies.deleteRecursively()
            throw e
        }
        return OpenedFiles(opened) { copies.deleteRecursively() }
    }

    private fun openOne(file: StagedFile, copies: File): OutgoingFile {
        val uri = file.uri.toUri()
        val modified = lastModified(uri)
        val pfd = resolver.openFileDescriptor(uri, "r") ?: error("${file.name} can't be opened")
        if (pfd.statSize >= 0 && isSeekable(pfd)) {
            return OutgoingFile(file.name, modified, FileSource.Fd(pfd.detachFd()))
        }
        pfd.close()
        copies.mkdirs()
        val copy = File(copies, "${nextIndex(copies)}-${file.name.replace('/', '_')}")
        resolver.openInputStream(uri).use { input ->
            checkNotNull(input) { "${file.name} can't be read" }
            copy.outputStream().use { input.copyTo(it, 1 shl 20) }
        }
        return OutgoingFile(file.name, modified, FileSource.Path(copy.path))
    }

    // Two shared files may have the same name.
    private fun nextIndex(dir: File) = dir.list()?.size ?: 0

    private fun isSeekable(pfd: ParcelFileDescriptor): Boolean = try {
        Os.lseek(pfd.fileDescriptor, 0, OsConstants.SEEK_CUR)
        true
    } catch (_: ErrnoException) {
        false
    }

    private fun lastModified(uri: Uri): Long = runCatching {
        resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_LAST_MODIFIED), null, null, null)?.use { c ->
            if (c.moveToFirst() && !c.isNull(0)) c.getLong(0) / 1000 else null
        }
    }.getOrNull() ?: (System.currentTimeMillis() / 1000)
}
