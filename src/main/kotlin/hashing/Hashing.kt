package hashing

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import java.util.zip.ZipInputStream

object Hashing {

    private const val hex255 = 0xff
    private const val hex256 = 0x100

    @Throws(IOException::class, IllegalArgumentException::class, IllegalStateException::class)
    suspend fun File.hash(
        digest: MessageDigest = Algorithms.SHA256,
        hashProgressCallback: (Float) -> Unit = {}
    ): String {
        withContext(Dispatchers.IO) {
            FileInputStream(this@hash).use {
                val buffer = ByteArray(size = 32_768)
                var bytesCount: Int

                val totalRuns = ((length() / buffer.size) + 1).toFloat()
                var count = 0
                while (withContext(Dispatchers.IO) { it.read(buffer) }.also { bytesCount = it } != -1) {
                    digest.update(buffer, 0, bytesCount)
                    hashProgressCallback(count++ / totalRuns)
                }
                hashProgressCallback(count / totalRuns)
            }
        }
        return buildHash(digest.digest())
    }

    fun File.hashMsixSignature(digest: MessageDigest = Algorithms.SHA256): String {
        val validExtensions = listOf("appx", "appxbundle", "msix", "msixbundle")
        require(extension.lowercase() in validExtensions) {
            "File extension must be one of the following: ${validExtensions.joinToString()}"
        }
        ZipInputStream(inputStream()).use { zip ->
            var entry = zip.nextEntry
            while (entry != null) {
                if (entry.name == "AppxSignature.p7x") {
                    updateDigest(zip, digest)
                }
                entry = zip.nextEntry
            }
        }
        return buildHash(digest.digest())
    }

    fun updateDigest(inputStream: InputStream, digest: MessageDigest) {
        val buffer = ByteArray(size = 1_024)
        var bytesCount = inputStream.read(buffer)
        while (bytesCount > 0) {
            digest.update(buffer, 0, bytesCount)
            bytesCount = inputStream.read(buffer)
        }
    }

    fun buildHash(bytes: ByteArray) = buildString {
        bytes.forEach { byte ->
            append(((byte.toInt() and hex255) + hex256).toString(radix = 16).substring(startIndex = 1))
        }
    }

    object Algorithms {
        private const val SHA_256 = "SHA-256"
        val SHA256: MessageDigest = MessageDigest.getInstance(SHA_256)
    }
}
