package dev.beam.android.data

import android.content.ContentValues
import android.content.Context
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import android.webkit.MimeTypeMap
import java.io.File

/**
 * Moves finished downloads from app-private storage to `Download/Beam`
 * through MediaStore (scoped storage: no storage permission needed).
 *
 * Core receives into app-private storage first because it needs positional
 * writes, a `.beam-partial` manifest and renames for resume, none of which a
 * MediaStore URI offers. The extra copy happens once, after the hash check.
 */
class DownloadsSaver(private val context: Context) {
    /** Returns a `content://` URI for each file saved; failures stay where they are. */
    fun save(paths: List<String>): List<String> = paths.mapNotNull { path ->
        runCatching { saveOne(File(path)) }
            .onFailure { Log.w(TAG, "could not move $path to Downloads", it) }
            .getOrNull()
    }

    private fun saveOne(file: File): String {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, file.name)
            put(MediaStore.Downloads.MIME_TYPE, mimeType(file.name))
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Beam")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: error("MediaStore refused ${file.name}")
        try {
            resolver.openOutputStream(uri).use { out ->
                checkNotNull(out) { "can't write ${file.name}" }
                file.inputStream().use { it.copyTo(out, 1 shl 20) }
            }
            resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        file.delete()
        return uri.toString()
    }

    companion object {
        private const val TAG = "Beam"

        fun mimeType(name: String): String =
            MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
                ?: "application/octet-stream"
    }
}
