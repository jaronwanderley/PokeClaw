// Copyright 2026 PokeClaw (agents.io). All rights reserved.
// Licensed under the Apache License, Version 2.0.

package io.agents.pokeclaw.channel

import io.agents.pokeclaw.ClawApplication
import io.agents.pokeclaw.R
import io.agents.pokeclaw.TaskOrchestrator
import io.agents.pokeclaw.service.ClawAccessibilityService
import io.agents.pokeclaw.utils.KVUtils

/**
 * Channel initialization and message routing.
 * Responsible for reading local config to initialize channels and dispatching received messages to [TaskOrchestrator].
 */
class ChannelSetup(
    private val taskOrchestrator: TaskOrchestrator
) {

    fun setup() {
        ChannelManager.init(
            discordBotToken = KVUtils.getDiscordBotToken().ifEmpty { null },
            telegramBotToken = KVUtils.getTelegramBotToken().ifEmpty { null },
            wechatBotToken = KVUtils.getWechatBotToken().ifEmpty { null },
            wechatApiBaseUrl = KVUtils.getWechatApiBaseUrl().ifEmpty { null }
        )
        ChannelManager.setOnMessageReceivedListener(object : ChannelManager.OnMessageReceivedListener {
            override fun onMessageReceived(channel: Channel, message: String, messageID: String) {
                val app = ClawApplication.instance
                if (!ClawAccessibilityService.isEnabledInSettings(app)) {
                    // Not enabled at all — tell user to enable, no point waiting
                    ChannelManager.sendMessage(channel, "Accessibility service is not enabled. Please enable PokeClaw in Settings > Accessibility.", messageID)
                    ChannelManager.flushMessages(channel)
                    return
                }
                if (!ClawAccessibilityService.awaitRunning(3000)) {
                    // Enabled but not connected after 3s — may need restart
                    ChannelManager.sendMessage(channel, app.getString(R.string.channel_msg_no_accessibility), messageID)
                    ChannelManager.flushMessages(channel)
                    return
                }
                if (!taskOrchestrator.tryAcquireTask(messageID, channel, message)) {
                    ChannelManager.sendMessage(channel, app.getString(R.string.channel_msg_task_in_progress), messageID)
                    ChannelManager.flushMessages(channel)
                    return
                }
                taskOrchestrator.startNewTask(channel, message, messageID)
            }
        })
    }
}
