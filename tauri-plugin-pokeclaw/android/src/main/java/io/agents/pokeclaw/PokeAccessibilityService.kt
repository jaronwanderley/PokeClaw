// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Context
import android.graphics.Path
import android.graphics.Rect
import android.provider.Settings
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Core accessibility service providing device interaction capabilities for the PokeClaw Tauri plugin.
 * Singleton-pattern: the running instance is accessible via [getInstance].
 */
class PokeAccessibilityService : AccessibilityService() {

    companion object {
        private const val TAG = "PokeA11yService"

        @Volatile
        private var instance: PokeAccessibilityService? = null

        /** Returns the running service instance, or null if not connected. */
        @JvmStatic
        fun getInstance(): PokeAccessibilityService? = instance

        /** Returns true if the service is currently connected. */
        @JvmStatic
        fun isRunning(): Boolean = instance != null

        /**
         * Returns the connected instance if available. If Android has the service enabled
         * but it is momentarily rebinding, waits up to [timeoutMs] milliseconds.
         *
         * Returns null if the service is not enabled in settings or the wait times out.
         */
        @JvmStatic
        fun getConnectedInstance(timeoutMs: Long): PokeAccessibilityService? {
            instance?.let { return it }

            // In a plugin context we don't have a direct Application reference,
            // so we cannot check isEnabledInSettings without a context.
            // Just wait for the instance to appear.
            Log.w(TAG, "Accessibility service not attached yet, waiting up to ${timeoutMs}ms for rebind")
            return if (awaitRunning(timeoutMs)) instance else null
        }

        /**
         * Checks whether PokeClaw is enabled in Android's Accessibility settings.
         * This does NOT mean the service is connected — use [isRunning] for that.
         */
        @JvmStatic
        fun isEnabledInSettings(context: Context): Boolean {
            return try {
                val enabledServices = Settings.Secure.getString(
                    context.contentResolver,
                    Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES
                ) ?: return false
                if (enabledServices.isEmpty()) return false
                val myService = context.packageName + "/" + PokeAccessibilityService::class.java.name
                enabledServices.contains(myService)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to check accessibility settings", e)
                false
            }
        }

        /**
         * Waits up to [timeoutMs] milliseconds for the service to connect.
         */
        @JvmStatic
        fun awaitRunning(timeoutMs: Long): Boolean {
            if (instance != null) return true
            val deadline = System.currentTimeMillis() + timeoutMs
            while (System.currentTimeMillis() < deadline) {
                try {
                    Thread.sleep(200)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                    return false
                }
                if (instance != null) return true
            }
            return false
        }
    }

    /** Node ID → center coordinates mapping for tap_node tool */
    private val nodeIdMap = ConcurrentHashMap<String, IntArray>()

    /** Monotonic counter for node IDs within a single getScreenTree call */
    private val nodeCounter = AtomicInteger(0)

    // ======================== Lifecycle ========================

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        Log.i(TAG, "Accessibility service connected")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // Debug: log notification events from messaging apps
        if (event != null && event.eventType == AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED) {
            Log.d(TAG, "Notification event from: ${event.packageName}")
        }
    }

    override fun onInterrupt() {
        Log.w(TAG, "Accessibility service interrupted")
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
        Log.i(TAG, "Accessibility service destroyed")
    }

    // ======================== Screen Tree ========================

    /**
     * Collects a compact tree representation of the current screen for AI analysis.
     * Format: `[n1] "text" tap edit (cx,cy)`
     *
     * Clears nodeIdMap and resets nodeCounter at the start of each call.
     * Skips invisible nodes but still traverses their children (matching legacy behavior).
     *
     * @return the screen tree string, or null if no root window is available.
     */
    fun getScreenTree(): String? {
        val root = rootInActiveWindow ?: return null
        nodeIdMap.clear()
        nodeCounter.set(0)
        val sb = StringBuilder()
        buildNodeTree(root, sb, 0)
        return sb.toString()
    }

    /** Get center coordinates for a node ID (e.g. "n3"). Returns null if not found. */
    fun getNodeCoordinates(nodeId: String): IntArray? = nodeIdMap[nodeId]

    /**
     * Finds all nodes matching the given text in the active window.
     * Caller is responsible for recycling the returned nodes via [recycleNodes].
     */
    fun findNodesByText(text: String): List<AccessibilityNodeInfo> {
        val root = rootInActiveWindow ?: return emptyList()
        val nodes = root.findAccessibilityNodeInfosByText(text)
        return nodes ?: emptyList()
    }

    /**
     * Returns detailed info about a single node as a human-readable string.
     */
    fun getNodeDetail(node: AccessibilityNodeInfo?): String {
        if (node == null) return "null"
        val sb = StringBuilder()
        sb.append("class=").append(node.className)
        node.text?.let { sb.append(", text=\"").append(it).append("\"") }
        node.contentDescription?.let { sb.append(", desc=\"").append(it).append("\"") }
        sb.append(", clickable=").append(node.isClickable)
        sb.append(", enabled=").append(node.isEnabled)
        sb.append(", visible=").append(node.isVisibleToUser)
        val bounds = Rect()
        node.getBoundsInScreen(bounds)
        sb.append(", bounds=").append(bounds.toShortString())
        return sb.toString()
    }

    /**
     * Recycles a list of AccessibilityNodeInfo nodes.
     * Call this after you are done using nodes returned by findNodesByText.
     */
    companion object {
        @JvmStatic
        fun recycleNodes(nodes: List<AccessibilityNodeInfo>?) {
            if (nodes == null) return
            for (node in nodes) {
                try {
                    node.recycle()
                } catch (_: Exception) {
                    // Already recycled
                }
            }
        }
    }

    // ======================== Gesture Dispatch ========================

    /**
     * Performs a tap gesture at the specified coordinates.
     *
     * @param x X coordinate in screen pixels.
     * @param y Y coordinate in screen pixels.
     * @param durationMs Touch duration in milliseconds (default 100).
     * @return true if the gesture completed successfully, false otherwise.
     */
    fun performTap(x: Int, y: Int, durationMs: Long = 100): Boolean {
        Log.d(TAG, "performTap: x=$x, y=$y, durationMs=$durationMs")
        val path = Path().apply { moveTo(x.toFloat(), y.toFloat()) }
        val stroke = GestureDescription.StrokeDescription(path, 0, durationMs)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        val result = dispatchGestureSync(gesture)
        Log.d(TAG, "performTap result: $result")
        return result
    }

    /**
     * Performs a swipe gesture from one point to another.
     *
     * @param startX Starting X coordinate.
     * @param startY Starting Y coordinate.
     * @param endX Ending X coordinate.
     * @param endY Ending Y coordinate.
     * @param durationMs Swipe duration in milliseconds.
     * @return true if the gesture completed successfully, false otherwise.
     */
    fun performSwipe(startX: Int, startY: Int, endX: Int, endY: Int, durationMs: Long): Boolean {
        Log.d(TAG, "performSwipe: ($startX,$startY) → ($endX,$endY), durationMs=$durationMs")
        val path = Path().apply {
            moveTo(startX.toFloat(), startY.toFloat())
            lineTo(endX.toFloat(), endY.toFloat())
        }
        val stroke = GestureDescription.StrokeDescription(path, 0, durationMs)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        val result = dispatchGestureSync(gesture)
        Log.d(TAG, "performSwipe result: $result")
        return result
    }

    /**
     * Performs a long-press gesture at the specified coordinates.
     *
     * @param x X coordinate in screen pixels.
     * @param y Y coordinate in screen pixels.
     * @param durationMs Press duration in milliseconds (default 1000).
     * @return true if the gesture completed successfully, false otherwise.
     */
    fun performLongPress(x: Int, y: Int, durationMs: Long): Boolean {
        Log.d(TAG, "performLongPress: x=$x, y=$y, durationMs=$durationMs")
        val path = Path().apply { moveTo(x.toFloat(), y.toFloat()) }
        val stroke = GestureDescription.StrokeDescription(path, 0, durationMs)
        val gesture = GestureDescription.Builder().addStroke(stroke).build()
        val result = dispatchGestureSync(gesture)
        Log.d(TAG, "performLongPress result: $result")
        return result
    }

    /**
     * Dispatches a gesture synchronously using CountDownLatch.
     * Blocks the calling thread until the gesture completes, is cancelled, or times out (5 seconds).
     *
     * @param gesture The gesture description to dispatch.
     * @return true if the gesture completed successfully, false on cancellation or timeout.
     */
    private fun dispatchGestureSync(gesture: GestureDescription): Boolean {
        val succeeded = AtomicBoolean(false)
        val latch = CountDownLatch(1)

        val callback = object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) {
                succeeded.set(true)
                latch.countDown()
            }

            override fun onCancelled(gestureDescription: GestureDescription?) {
                Log.w(TAG, "dispatchGestureSync: gesture cancelled")
                succeeded.set(false)
                latch.countDown()
            }
        }

        val dispatched = dispatchGesture(gesture, callback, null)
        if (!dispatched) {
            Log.e(TAG, "dispatchGestureSync: dispatchGesture returned false (system rejected)")
            return false
        }

        return try {
            val completed = latch.await(5, TimeUnit.SECONDS)
            if (!completed) {
                Log.e(TAG, "dispatchGestureSync: timed out after 5 seconds")
                false
            } else {
                succeeded.get()
            }
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
            Log.e(TAG, "dispatchGestureSync: interrupted while waiting", e)
            false
        }
    }

    /**
     * Returns the screen dimensions in pixels.
     * Uses [android.util.DisplayMetrics] from resources — no external ScreenUtils dependency.
     *
     * @return IntArray of [widthPixels, heightPixels].
     */
    fun getScreenSize(): IntArray {
        val metrics = resources.displayMetrics
        val w = metrics.widthPixels
        val h = metrics.heightPixels
        Log.d(TAG, "getScreenSize: ${w}x${h}")
        return intArrayOf(w, h)
    }

    // ======================== Internal: Tree Builder ========================

    private fun buildNodeTree(node: AccessibilityNodeInfo, sb: StringBuilder, depth: Int) {
        // Skip nodes not visible on screen
        if (!node.isVisibleToUser) {
            // Still traverse children — invisible parent doesn't mean all children are invisible
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                buildNodeTree(child, sb, depth)
                child.recycle()
            }
            return
        }

        // Determine whether the current node is "meaningful"
        val hasText = node.text != null && node.text.isNotEmpty()
        val hasDesc = node.contentDescription != null && node.contentDescription.isNotEmpty()
        val isInteractive = node.isClickable || node.isScrollable || node.isEditable
                || node.isCheckable || node.isLongClickable
        val isSlider = isSliderNode(node)
        val isProgress = node.className?.toString()?.contains("ProgressBar") == true
        val isMeaningful = hasText || hasDesc || isInteractive || isSlider || isProgress

        if (isMeaningful) {
            val bounds = Rect()
            node.getBoundsInScreen(bounds)
            val cx = (bounds.left + bounds.right) / 2
            val cy = (bounds.top + bounds.bottom) / 2

            val nodeId = "n${nodeCounter.incrementAndGet()}"
            nodeIdMap[nodeId] = intArrayOf(cx, cy)

            // Format: [n1] "text" tap edit (cx,cy)
            val line = StringBuilder()
            for (d in 0 until minOf(depth, 4)) line.append("  ")

            line.append("[").append(nodeId).append("] ")

            if (hasText) {
                val text = node.text!!
                line.append("\"")
                    .append(if (text.length > 40) "${text.substring(0, 40)}.." else text)
                    .append("\"")
            } else if (hasDesc) {
                line.append("\"").append(node.contentDescription).append("\"")
            }

            if (node.isClickable) line.append(" tap")
            if (node.isEditable) line.append(" edit")
            if (node.isScrollable) line.append(" scroll")
            if (node.isCheckable) line.append(if (node.isChecked) " on" else " off")

            line.append(" (").append(cx).append(",").append(cy).append(")")

            sb.append(line).append("\n")
        }

        // Children: if current node was skipped (not meaningful), they keep the same depth
        val childDepth = if (isMeaningful) depth + 1 else depth
        for (i in 0 until node.childCount) {
            val child = node.getChild(i) ?: continue
            buildNodeTree(child, sb, childDepth)
            child.recycle()
        }
    }

    private fun isSliderNode(node: AccessibilityNodeInfo): Boolean {
        val className = node.className?.toString() ?: return false
        return className.contains("SeekBar")
                || className.contains("Slider")
                || className.contains("RatingBar")
                || node.rangeInfo != null
    }
}
