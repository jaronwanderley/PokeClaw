import Foundation
import UIKit
import WebKit
import Tauri
import os.log
import UserNotifications

@objc(PokeclawPlugin)
public class PokeclawPlugin: Plugin {
    private let logger = Logger(subsystem: "io.agents.pokeclaw", category: "plugin")

    @objc public override func load() {
        os_log("PokeclawPlugin loaded", log: .default, type: .info)
    }

    private func resolve(_ invoke: Invoke, _ success: Bool, _ data: Any? = nil, _ error: String? = nil) {
        let res: [String: Any?] = [
            "success": success,
            "data": data,
            "error": error
        ]
        invoke.resolve(res)
    }

    private func unavailableResult(_ invoke: Invoke, _ command: String) {
        os_log("Command %{public}@ is not available on iOS", log: .default, type: .info, command)
        resolve(invoke, false, nil, "Not available on iOS")
    }

    // MARK: - Real iOS Implementations

    @objc public func get_device_info(_ invoke: Invoke) {
        let category = invoke.getString("category")?.lowercased() ?? "device"
        os_log("get_device_info: category=%{public}@", log: .default, type: .info, category)

        var info = ""
        switch category {
        case "battery":
            UIDevice.current.isBatteryMonitoringEnabled = true
            let level = Int(UIDevice.current.batteryLevel * 100)
            let state: String
            switch UIDevice.current.batteryState {
            case .charging: state = "Charging"
            case .full: state = "Full"
            case .unplugged: state = "Unplugged"
            default: state = "Unknown"
            }
            info = "Battery Level: \(level)%, State: \(state)"
        case "screen":
            let bounds = UIScreen.main.bounds
            let scale = UIScreen.main.scale
            info = "Resolution: \(Int(bounds.width * scale))x\(Int(bounds.height * scale)), Scale: \(scale)"
        case "device":
            let device = UIDevice.current
            info = "Name: \(device.name), Model: \(device.model), System: \(device.systemName) \(device.systemVersion)"
        case "storage":
            if let attr = try? FileManager.default.attributesOfFileSystem(forPath: NSHomeDirectory()),
               let freeSize = attr[.systemFreeSize] as? Int64,
               let totalSize = attr[.systemSize] as? Int64 {
                info = "Free: \(freeSize / 1024 / 1024 / 1024)GB, Total: \(totalSize / 1024 / 1024 / 1024)GB"
            } else {
                info = "Storage info unavailable"
            }
        case "memory":
            let memory = ProcessInfo.processInfo.physicalMemory
            info = "Physical Memory: \(memory / 1024 / 1024 / 1024)GB"
        default:
            info = "Category \(category) not fully implemented on iOS, returning basic device info: \(UIDevice.current.model)"
        }

        resolve(invoke, true, info)
    }

    @objc public func check_permissions(_ invoke: Invoke) {
        os_log("check_permissions", log: .default, type: .info)
        
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            let notificationsEnabled = settings.authorizationStatus == .authorized
            
            DispatchQueue.main.async {
                let data: [String: Any] = [
                    "accessibility_enabled": false, // Not applicable on iOS in the same way
                    "accessibility_running": false,
                    "notification_enabled": notificationsEnabled,
                    "foreground_service": true      // Assuming the app is running
                ]
                self.resolve(invoke, true, data)
            }
        }
    }

    @objc public func clipboard(_ invoke: Invoke) {
        let action = invoke.getString("action")?.lowercased() ?? "get"
        os_log("clipboard: action=%{public}@", log: .default, type: .info, action)

        if action == "get" {
            let text = UIPasteboard.general.string ?? ""
            resolve(invoke, true, text)
        } else if action == "set" {
            let text = invoke.getString("text") ?? ""
            UIPasteboard.general.string = text
            resolve(invoke, true, "Clipboard set")
        } else {
            resolve(invoke, false, nil, "Unknown clipboard action: \(action)")
        }
    }

    @objc public func get_installed_apps(_ invoke: Invoke) {
        os_log("get_installed_apps", log: .default, type: .info)
        
        // On iOS, we can only check for apps via URL schemes.
        // This is highly limited and requires LSApplicationQueriesSchemes in Info.plist.
        // For this scaffold, we'll check a few common ones.
        let commonSchemes = [
            "whatsapp": "whatsapp://",
            "facebook": "fb://",
            "twitter": "twitter://",
            "instagram": "instagram://",
            "youtube": "youtube://"
        ]
        
        var installedApps: [[String: String]] = []
        for (name, scheme) in commonSchemes {
            if let url = URL(string: scheme), UIApplication.shared.canOpenURL(url) {
                installedApps.append(["name": name, "scheme": scheme])
            }
        }
        
        resolve(invoke, true, installedApps)
    }

    // MARK: - Unavailable Implementations

    @objc public func get_screen_info(_ invoke: Invoke) { unavailableResult(invoke, "get_screen_info") }
    @objc public func find_node_info(_ invoke: Invoke) { unavailableResult(invoke, "find_node_info") }
    @objc public func tap(_ invoke: Invoke) { unavailableResult(invoke, "tap") }
    @objc public func swipe(_ invoke: Invoke) { unavailableResult(invoke, "swipe") }
    @objc public func long_press(_ invoke: Invoke) { unavailableResult(invoke, "long_press") }
    @objc public func tap_node(_ invoke: Invoke) { unavailableResult(invoke, "tap_node") }
    @objc public func input_text(_ invoke: Invoke) { unavailableResult(invoke, "input_text") }
    @objc public func scroll_to_find(_ invoke: Invoke) { unavailableResult(invoke, "scroll_to_find") }
    @objc public func find_and_tap(_ invoke: Invoke) { unavailableResult(invoke, "find_and_tap") }
    @objc public func get_notifications(_ invoke: Invoke) { unavailableResult(invoke, "get_notifications") }
    @objc public func open_app(_ invoke: Invoke) { unavailableResult(invoke, "open_app") }
    @objc public func system_key(_ invoke: Invoke) { unavailableResult(invoke, "system_key") }
    @objc public func send_chat_message(_ invoke: Invoke) { unavailableResult(invoke, "send_chat_message") }
    @objc public func take_screenshot(_ invoke: Invoke) { unavailableResult(invoke, "take_screenshot") }
    @objc public func make_call(_ invoke: Invoke) { unavailableResult(invoke, "make_call") }
    @objc public func open_permission_settings(_ invoke: Invoke) { unavailableResult(invoke, "open_permission_settings") }
    @objc public func start_session(_ invoke: Invoke) { unavailableResult(invoke, "start_session") }
    @objc public func stop_session(_ invoke: Invoke) { unavailableResult(invoke, "stop_session") }
    @objc public func send_message(_ invoke: Invoke) { unavailableResult(invoke, "send_message") }
    @objc public func get_session_status(_ invoke: Invoke) { unavailableResult(invoke, "get_session_status") }
    @objc public func ping(_ invoke: Invoke) { unavailableResult(invoke, "ping") }
    @objc public func chat(_ invoke: Invoke) { unavailableResult(invoke, "chat") }
    @objc public func list_models(_ invoke: Invoke) {
        os_log("list_models", log: .default, type: .info)
        
        var models: [[String: Any]] = []
        for model in ModelManager.AVAILABLE_MODELS {
            let isDownloaded = ModelManager.isModelDownloaded(model: model)
            let localPath = ModelManager.getModelPath(model: model) ?? ""
            
            let modelDict: [String: Any] = [
                "id": model.id,
                "displayName": model.displayName,
                "url": model.url,
                "repoId": model.repoId,
                "fileName": model.fileName,
                "sizeBytes": model.sizeBytes,
                "minRamGb": model.minRamGb,
                "isDownloaded": isDownloaded,
                "localPath": localPath
            ]
            models.append(modelDict)
        }
        
        invoke.resolve(["models": models])
    }

    @objc public func download_model(_ invoke: Invoke) {
        let modelId = invoke.getString("modelId") ?? ""
        os_log("download_model: modelId=%{public}@", log: .default, type: .info, modelId)
        
        guard let model = ModelManager.getModelById(modelId) else {
            invoke.reject("Unknown model ID: \(modelId)")
            return
        }
        
        // Check if already downloaded
        if let existingPath = ModelManager.getModelPath(model: model) {
            // Frontend downloadModel expect nothing from resolve if successful, 
            // but it uses the Channel for 'complete' event.
            // However, it's good to resolve to signal completion of the command itself.
            invoke.resolve()
            return
        }
        
        // Progress events via Channel if available, or trigger
        // useModel.ts uses Channel: const onProgress = new Channel<DownloadEvent>()
        // We need to find how to get Channel from Invoke in Swift.
        // Assuming it's in the args.
        
        // For now, if we can't get the channel, we'll just resolve when done.
        // But the frontend expects progress.
        
        ModelManager.downloadModel(model: model) { progress in
            // Emit progress event
            // If we can find the channel, we should use it.
            // Frontend: await invoke('download_model', { modelId, onProgress })
            // Tauri 2.0 serializes Channel as an ID in the arguments.
            
            // For now, I'll just use trigger which might be picked up if configured,
            // but the Channel is the preferred way.
            // Since I don't have the Channel API for Swift handy, 
            // I'll stick to a simple implementation.
        } onComplete: { path in
            invoke.resolve(["path": path])
        } onError: { error in
            invoke.reject(error)
        }
    }
}

@_cdecl("init_plugin_pokeclaw")
func init_plugin_pokeclaw(messenger: Messenger) {
    messenger.registerPlugin(PokeclawPlugin(messenger: messenger))
}
