// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log

/**
 * Notification listener service that captures incoming notifications (WhatsApp, Telegram, etc.)
 * and makes them available to the PokeClaw plugin for agent-driven task execution.
 *
 * Singleton-pattern: the running instance is accessible via [getInstance].
 * Notifications are broadcast locally so any component can observe them.
 */
class PokeNotificationListener : NotificationListenerService() {

    companion object {
        private const val TAG = "PokeNotifListener"

        /** Local broadcast action when a new notification is posted */
        const val ACTION_NOTIFICATION_POSTED = "io.agents.pokeclaw.NOTIFICATION_POSTED"
        /** Local broadcast action when a notification is removed */
        const val ACTION_NOTIFICATION_REMOVED = "io.agents.pokeclaw.NOTIFICATION_REMOVED"

        /** Extra keys for broadcast intents */
        const val EXTRA_PACKAGE_NAME = "package_name"
        const val EXTRA_TICKER_TEXT = "ticker_text"
        const val EXTRA_POST_TIME = "post_time"
        const val EXTRA_KEY = "notification_key"

        @Volatile
        private var instance: PokeNotificationListener? = null

        /** Returns the running service instance, or null if not connected. */
        @JvmStatic
        fun getInstance(): PokeNotificationListener? = instance

        /** Returns true if the notification listener service is currently connected. */
        @JvmStatic
        fun isRunning(): Boolean = instance != null
    }

    // ======================== Lifecycle ========================

    override fun onListenerConnected() {
        super.onListenerConnected()
        instance = this
        Log.i(TAG, "Notification listener connected")
    }

    override fun onListenerDisconnected() {
        instance = null
        Log.i(TAG, "Notification listener disconnected")
        super.onListenerDisconnected()
    }

    override fun onDestroy() {
        instance = null
        Log.i(TAG, "Notification listener destroyed")
        super.onDestroy()
    }

    // ======================== Notification Events ========================

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null) return

        val packageName = sbn.packageName ?: return
        val tickerText = sbn.notification?.tickerText?.toString() ?: ""
        val postTime = sbn.postTime
        val key = sbn.key ?: ""

        Log.d(TAG, "Notification posted: package=$packageName, ticker=\"$tickerText\", key=$key")

        // Broadcast locally for other components
        val intent = Intent(ACTION_NOTIFICATION_POSTED).apply {
            putExtra(EXTRA_PACKAGE_NAME, packageName)
            putExtra(EXTRA_TICKER_TEXT, tickerText)
            putExtra(EXTRA_POST_TIME, postTime)
            putExtra(EXTRA_KEY, key)
            setPackage(packageName)
        }
        sendBroadcast(intent)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        if (sbn == null) return

        val packageName = sbn.packageName ?: return
        val key = sbn.key ?: ""

        Log.d(TAG, "Notification removed: package=$packageName, key=$key")

        val intent = Intent(ACTION_NOTIFICATION_REMOVED).apply {
            putExtra(EXTRA_PACKAGE_NAME, packageName)
            putExtra(EXTRA_KEY, key)
            setPackage(packageName)
        }
        sendBroadcast(intent)
    }

    // ======================== Public API ========================

    /**
     * Returns all currently active notifications.
     * Each item contains: packageName, postTime, key, tickerText.
     *
     * @return List of notification data maps, or empty list if none.
     */
    fun getActiveNotificationsList(): List<Map<String, Any?>> {
        val sbns = try {
            getActiveNotifications() ?: emptyArray()
        } catch (e: SecurityException) {
            Log.e(TAG, "Failed to get active notifications (security exception)", e)
            return emptyList()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to get active notifications", e)
            return emptyList()
        }

        return sbns.mapNotNull { sbn ->
            try {
                mapOf(
                    "package_name" to (sbn.packageName ?: ""),
                    "key" to (sbn.key ?: ""),
                    "post_time" to sbn.postTime,
                    "ticker_text" to (sbn.notification?.tickerText?.toString() ?: ""),
                    "is_ongoing" to (sbn.isOngoing),
                    "is_clearable" to (sbn.isClearable)
                )
            } catch (e: Exception) {
                Log.w(TAG, "Failed to read notification data for ${sbn.key}", e)
                null
            }
        }
    }

    /**
     * Dismisses a notification by its key.
     *
     * @param key The StatusBarNotification key.
     * @return true if the notification was dismissed, false otherwise.
     */
    fun dismissNotification(key: String): Boolean {
        return try {
            cancelNotification(key)
            Log.d(TAG, "Dismissed notification: key=$key")
            true
        } catch (e: SecurityException) {
            Log.e(TAG, "Failed to dismiss notification (security exception): key=$key", e)
            false
        } catch (e: Exception) {
            Log.e(TAG, "Failed to dismiss notification: key=$key", e)
            false
        }
    }

    /**
     * Dismisses all notifications.
     *
     * @return true if successful, false otherwise.
     */
    fun dismissAllNotifications(): Boolean {
        return try {
            cancelAllNotifications()
            Log.d(TAG, "Dismissed all notifications")
            true
        } catch (e: SecurityException) {
            Log.e(TAG, "Failed to dismiss all notifications (security exception)", e)
            false
        } catch (e: Exception) {
            Log.e(TAG, "Failed to dismiss all notifications", e)
            false
        }
    }
}
