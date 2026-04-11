// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.os.SystemClock
import android.util.Log

/**
 * Foreground service that keeps the PokeClaw app alive while agent tasks are running.
 * Displays a persistent notification so the user knows the service is active.
 *
 * Singleton-pattern: the running instance is accessible via [getInstance].
 */
class PokeForegroundService : Service() {

    companion object {
        private const val TAG = "PokeFgService"
        private const val NOTIFICATION_CHANNEL_ID = "pokeclaw_foreground"
        private const val NOTIFICATION_ID = 1001
        private const val ACTION_START = "io.agents.pokeclaw.action.START_FOREGROUND"
        private const val ACTION_STOP = "io.agents.pokeclaw.action.STOP_FOREGROUND"
        private const val ACTION_RESTART = "io.agents.pokeclaw.action.RESTART_FOREGROUND"

        /** How often to attempt restart if the service is killed (ms) */
        private const val RESTART_INTERVAL_MS = 5_000L

        @Volatile
        private var instance: PokeForegroundService? = null

        @Volatile
        private var shouldRestartOnDestroy = false

        /** Returns the running service instance, or null if not running. */
        @JvmStatic
        fun getInstance(): PokeForegroundService? = instance

        /** Returns true if the foreground service is currently running. */
        @JvmStatic
        fun isRunning(): Boolean = instance != null

        /**
         * Starts the foreground service. Creates the notification channel and
         * starts the service as foreground.
         *
         * @param context Android context for starting the service.
         */
        @JvmStatic
        fun start(context: Context) {
            Log.i(TAG, "start: initiating foreground service")
            val intent = Intent(context, PokeForegroundService::class.java).apply {
                action = ACTION_START
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        /**
         * Stops the foreground service. Disables auto-restart before stopping.
         *
         * @param context Android context for stopping the service.
         */
        @JvmStatic
        fun stop(context: Context) {
            Log.i(TAG, "stop: stopping foreground service")
            shouldRestartOnDestroy = false
            val intent = Intent(context, PokeForegroundService::class.java).apply {
                action = ACTION_STOP
            }
            context.startService(intent)
        }

        /**
         * Schedules a restart alarm so the service can recover from being killed.
         */
        private fun scheduleRestartAlarm(context: Context) {
            val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as? AlarmManager
            if (alarmManager == null) {
                Log.w(TAG, "scheduleRestartAlarm: AlarmManager not available")
                return
            }

            val intent = Intent(context, PokeForegroundService::class.java).apply {
                action = ACTION_RESTART
            }
            val pendingIntent = PendingIntent.getService(
                context,
                0,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )

            val triggerAt = SystemClock.elapsedRealtime() + RESTART_INTERVAL_MS
            alarmManager.setAndAllowWhileIdle(
                AlarmManager.ELAPSED_REALTIME_WAKEUP,
                triggerAt,
                pendingIntent
            )
            Log.d(TAG, "scheduleRestartAlarm: restart scheduled in ${RESTART_INTERVAL_MS}ms")
        }

        /**
         * Cancels any pending restart alarm.
         */
        private fun cancelRestartAlarm(context: Context) {
            val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as? AlarmManager ?: return
            val intent = Intent(context, PokeForegroundService::class.java).apply {
                action = ACTION_RESTART
            }
            val pendingIntent = PendingIntent.getService(
                context,
                0,
                intent,
                PendingIntent.FLAG_NO_CREATE or PendingIntent.FLAG_IMMUTABLE
            )
            pendingIntent?.let {
                alarmManager.cancel(it)
                Log.d(TAG, "cancelRestartAlarm: cancelled pending restart alarm")
            }
        }
    }

    // ======================== Lifecycle ========================

    override fun onCreate() {
        super.onCreate()
        instance = this
        createNotificationChannel()
        Log.i(TAG, "Foreground service created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.d(TAG, "onStartCommand: action=${intent?.action}")

        when (intent?.action) {
            ACTION_START -> {
                shouldRestartOnDestroy = true
                val notification = buildNotification("PokeClaw agent is running")
                startForeground(NOTIFICATION_ID, notification)
                Log.i(TAG, "Foreground service started")
            }
            ACTION_STOP -> {
                shouldRestartOnDestroy = false
                cancelRestartAlarm(this)
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                Log.i(TAG, "Foreground service stopped")
            }
            ACTION_RESTART -> {
                shouldRestartOnDestroy = true
                val notification = buildNotification("PokeClaw agent is running (restarted)")
                startForeground(NOTIFICATION_ID, notification)
                Log.i(TAG, "Foreground service restarted via alarm")
            }
            else -> {
                // Default: start as foreground
                shouldRestartOnDestroy = true
                val notification = buildNotification("PokeClaw agent is running")
                startForeground(NOTIFICATION_ID, notification)
                Log.i(TAG, "Foreground service started (default action)")
            }
        }

        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        Log.i(TAG, "Foreground service destroyed, shouldRestart=$shouldRestartOnDestroy")
        if (shouldRestartOnDestroy) {
            scheduleRestartAlarm(this)
        }
        instance = null
        super.onDestroy()
    }

    // ======================== Notification ========================

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                "PokeClaw Agent",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Shows when PokeClaw agent is actively running tasks"
                setShowBadge(false)
                lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            }
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(channel)
            Log.d(TAG, "Notification channel created")
        }
    }

    private fun buildNotification(contentText: String): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
                .setPriority(Notification.PRIORITY_LOW)
        }

        return builder
            .setContentTitle("PokeClaw")
            .setContentText(contentText)
            .setSmallIcon(R.drawable.ic_notification)
            .setOngoing(true)
            .setShowWhen(false)
            .build()
    }
}
