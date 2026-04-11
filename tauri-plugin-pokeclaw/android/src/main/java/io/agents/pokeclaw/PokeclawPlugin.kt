package io.agents.pokeclaw

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke

@TauriPlugin
class PokeclawPlugin(private val activity: Activity): Plugin(activity) {
    @Command
    fun ping(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", "pong")
        invoke.resolve(ret)
    }
}
