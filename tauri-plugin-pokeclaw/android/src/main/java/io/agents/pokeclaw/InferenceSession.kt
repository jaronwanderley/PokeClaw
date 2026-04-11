package io.agents.pokeclaw

import android.util.Log
import com.google.ai.edge.litertlm.Conversation

/**
 * Wraps a LiteRT-LM Conversation with its metadata.
 *
 * LiteRT-LM Conversation is single-session — only one can exist per engine
 * at a time. This wrapper tracks state needed for session management.
 */
class InferenceSession(
    val conversation: Conversation,
    val modelPath: String,
    val backendLabel: String,
    val sessionId: String,
) {
    companion object {
        private const val TAG = "InferenceSession"
    }

    /** Number of messages processed so far in this conversation. */
    var processedMessageCount: Int = 0
        private set

    /** Number of sendMessage calls made (resets on conversation recreate). */
    var sendCount: Int = 0
        private set

    /** Increment after a successful sendMessage call. */
    fun recordSend() {
        sendCount++
        processedMessageCount++
    }

    /** Close the underlying conversation. Safe to call multiple times. */
    fun close() {
        Log.i(TAG, "close: sessionId=$sessionId, sendCount=$sendCount, processed=$processedMessageCount")
        try {
            conversation.close()
        } catch (e: Exception) {
            Log.w(TAG, "close: error closing conversation: ${e.message}")
        }
    }
}
