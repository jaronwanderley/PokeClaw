import Foundation
import UIKit
import WebKit
import Tauri
import os.log
import UserNotifications

@objc(PokeclawPlugin)
public class PokeclawPlugin: Plugin {
    private let logger = Logger(subsystem: "io.agents.pokeclaw", category: "plugin")
    
    // Active inference session and engine
    private var engine: CoreMLEngine?
    private var activeSession: InferenceSession?

    @objc public override func load() {
        os_log("PokeclawPlugin loaded", log: .default, type: .info)
    }

    private func resolve(_ invoke: Invoke, _ success: Bool, _ data: Any? = nil, _ error: String? = nil) {
        var res: [String: Any] = ["success": success]
        if let data = data { res["data"] = data }
        if let error = error { res["error"] = error }
        invoke.resolve(res)
    }

    private func unavailableResult(_ invoke: Invoke, _ command: String) {
        os_log("Command %{public}@ is not available on iOS", log: .default, type: .info, command)
        resolve(invoke, false, nil, "Not available on iOS")
    }

    // MARK: - Core Commands

    @objc public func ping(_ invoke: Invoke) {
        invoke.resolve(["value": "pong"])
    }

    @objc public func start_session(_ invoke: Invoke) {
        let modelId = invoke.getString("modelId") ?? invoke.getString("model_id") ?? ""
        let sessionId = invoke.getString("sessionId") ?? invoke.getString("session_id") ?? UUID().uuidString
        let computeUnitStr = invoke.getString("computeUnit") ?? "all"
        
        os_log("start_session: modelId=%{public}@, sessionId=%{public}@", log: .default, type: .info, modelId, sessionId)
        
        guard let model = ModelManager.getModelById(modelId) else {
            invoke.reject("Unknown model ID: \(modelId)")
            return
        }
        
        guard ModelManager.isModelDownloaded(model: model) else {
            invoke.reject("Model not downloaded")
            return
        }
        
        let computeUnit = CoreMLEngine.ComputeUnit(rawValue: computeUnitStr) ?? .all
        
        Task {
            do {
                let engine = try await CoreMLEngine(modelInfo: model, computeUnit: computeUnit)
                let session = InferenceSession(id: sessionId, engine: engine)
                self.activeSession = session
                self.engine = engine
                
                invoke.resolve([
                    "session_id": sessionId,
                    "backend": computeUnitStr
                ])
            } catch {
                invoke.reject("Failed to start session: \(error.localizedDescription)")
            }
        }
    }

    @objc public func stop_session(_ invoke: Invoke) {
        os_log("stop_session", log: .default, type: .info)
        activeSession?.stop()
        activeSession = nil
        engine = nil
        invoke.resolve(["success": true])
    }

    @objc public func send_message(_ invoke: Invoke) {
        guard let session = activeSession else {
            invoke.reject("No active session. Call start_session first.")
            return
        }
        
        let message = invoke.getString("message") ?? ""
        // batchSize is not directly used in InferenceSession yet but kept for contract parity
        let _ = invoke.getInt("batchSize") ?? 5
        
        guard let channel = invoke.getChannel("onEvent") else {
            invoke.reject("onEvent channel is required")
            return
        }
        
        os_log("send_message: starting generation", log: .default, type: .info)
        
        session.generate(
            prompt: message,
            onEvent: { event, data in
                // Match Android contract: {event: "token_batch", data: {tokens, batch_index}}
                let payload: [String: Any] = [
                    "event": event,
                    "data": data
                ]
                channel.send(payload)
            },
            onComplete: { fullText in
                let completeData: [String: Any] = [
                    "full_text": fullText,
                    "token_count": 0
                ]
                let payload: [String: Any] = [
                    "event": "complete",
                    "data": completeData
                ]
                channel.send(payload)
                invoke.resolve()
            },
            onError: { error in
                let errorData: [String: Any] = ["message": error]
                let payload: [String: Any] = [
                    "event": "error",
                    "data": errorData
                ]
                channel.send(payload)
                invoke.reject(error)
            }
        )
    }

    @objc public func get_session_status(_ invoke: Invoke) {
        let res = NSMutableDictionary()
        if let session = activeSession {
            res["state"] = "ready"
            res["session_id"] = session.id
            res["model_path"] = engine?.modelInfo.fileName ?? ""
            res["backend"] = engine?.modelInfo.repoId ?? ""
        } else {
            res["state"] = "idle"
        }
        invoke.resolve(res as? [String: Any] ?? [:])
    }

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
        let channel = invoke.getChannel("onProgress")
        
        os_log("download_model: modelId=%{public}@", log: .default, type: .info, modelId)
        
        guard let model = ModelManager.getModelById(modelId) else {
            invoke.reject("Unknown model ID: \(modelId)")
            return
        }
        
        if let existingPath = ModelManager.getModelPath(model: model) {
            if let channel = channel {
                let completeData: [String: Any] = [
                    "modelPath": existingPath,
                    "fileName": model.fileName
                ]
                channel.send(["event": "complete", "data": completeData])
            }
            invoke.resolve()
            return
        }
        
        ModelManager.downloadModel(model: model) { progress in
            if let channel = channel {
                let data: [String: Any] = [
                    "bytesDownloaded": Int64(progress * Double(model.sizeBytes)),
                    "totalBytes": model.sizeBytes,
                    "bytesPerSecond": 0
                ]
                channel.send(["event": "progress", "data": data])
            }
        } onComplete: { path in
            if let channel = channel {
                let data: [String: Any] = [
                    "modelPath": path,
                    "fileName": model.fileName
                ]
                channel.send(["event": "complete", "data": data])
            }
            invoke.resolve()
        } onError: { error in
            if let channel = channel {
                channel.send(["event": "error", "data": ["message": error]])
            }
            invoke.reject(error)
        }
    }

    // MARK: - Device Info & Permissions

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
            info = "Category \(category) not fully implemented on iOS"
        }

        resolve(invoke, true, ["info": info])
    }

    @objc public func check_permissions(_ invoke: Invoke) {
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            let notificationsEnabled = settings.authorizationStatus == .authorized
            DispatchQueue.main.async {
                let data: [String: Any] = [
                    "accessibility_enabled": false,
                    "accessibility_running": false,
                    "notification_enabled": notificationsEnabled,
                    "foreground_service": true
                ]
                self.resolve(invoke, true, data)
            }
        }
    }

    @objc public func clipboard(_ invoke: Invoke) {
        let action = invoke.getString("action")?.lowercased() ?? "get"
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
        let commonSchemes = ["whatsapp": "whatsapp://", "fb": "fb://", "twitter": "twitter://"]
        var installedApps: [[String: String]] = []
        for (name, scheme) in commonSchemes {
            if let url = URL(string: scheme), UIApplication.shared.canOpenURL(url) {
                installedApps.append(["name": name, "scheme": scheme])
            }
        }
        resolve(invoke, true, ["apps": installedApps])
    }

    // MARK: - Live Activity Commands (Dynamic Island)

    @objc public func start_live_activity(_ invoke: Invoke) {
        let title = invoke.getString("title") ?? ""
        os_log("start_live_activity: title=%{public}@", log: .default, type: .info, title)

        guard !title.isEmpty else {
            resolve(invoke, false, nil, "title parameter is required")
            return
        }

        if #available(iOS 16.1, *) {
            let started = LiveActivityManager.shared.startActivity(title: title)
            if started {
                resolve(invoke, true)
            } else {
                resolve(invoke, false, nil, "Failed to start Live Activity. Ensure Live Activities are enabled in device settings.")
            }
        } else {
            resolve(invoke, false, nil, "Live Activities require iOS 16.1 or later")
        }
    }

    @objc public func update_live_activity(_ invoke: Invoke) {
        let step = invoke.getInt("step") ?? 0
        let totalSteps = invoke.getInt("totalSteps") ?? 0
        let description = invoke.getString("description") ?? ""
        let status = invoke.getString("status") ?? "running"

        os_log("update_live_activity: step=%d/%d, status=%{public}@", log: .default, type: .info, step, totalSteps, status)

        if #available(iOS 16.1, *) {
            LiveActivityManager.shared.updateActivity(
                step: step,
                totalSteps: totalSteps,
                description: description,
                status: status
            )
            resolve(invoke, true)
        } else {
            resolve(invoke, false, nil, "Live Activities require iOS 16.1 or later")
        }
    }

    @objc public func stop_live_activity(_ invoke: Invoke) {
        os_log("stop_live_activity", log: .default, type: .info)

        if #available(iOS 16.1, *) {
            LiveActivityManager.shared.stopActivity()
            resolve(invoke, true)
        } else {
            resolve(invoke, false, nil, "Live Activities require iOS 16.1 or later")
        }
    }

    // MARK: - Legacy / Unavailable Implementations

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
    @objc public func chat(_ invoke: Invoke) { unavailableResult(invoke, "chat") }
}

@_cdecl("init_plugin_pokeclaw")
func init_plugin_pokeclaw(messenger: Messenger) {
    messenger.registerPlugin(PokeclawPlugin(messenger: messenger))
}
