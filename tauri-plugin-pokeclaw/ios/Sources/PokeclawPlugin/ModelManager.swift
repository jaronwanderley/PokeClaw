import Foundation
import Hub
import Transformers
import os.log

/**
 * Manages on-device LLM model downloads and storage for iOS.
 *
 * Models are downloaded from HuggingFace using swift-transformers Hub module
 * and stored in the app's document directory.
 */
public class ModelManager {
    private static let logger = Logger(subsystem: "io.agents.pokeclaw", category: "ModelManager")
    
    public struct ModelInfo: Codable {
        public let id: String
        public let displayName: String
        public let repoId: String
        public let url: String
        public let fileName: String // Main model file (e.g. .mlpackage)
        public let sizeBytes: Int64
        public let minRamGb: Int
        
        public init(id: String, displayName: String, repoId: String, url: String, fileName: String, sizeBytes: Int64, minRamGb: Int) {
            self.id = id
            self.displayName = displayName
            self.repoId = repoId
            self.url = url
            self.fileName = fileName
            self.sizeBytes = sizeBytes
            self.minRamGb = minRamGb
        }
    }
    
    public static let AVAILABLE_MODELS = [
        ModelInfo(
            id: "gemma4-e2b",
            displayName: "Gemma 4 E2B — 2.6GB",
            repoId: "google/gemma-4-E2B-it",
            url: "https://huggingface.co/google/gemma-4-E2B-it",
            fileName: "model.mlpackage",
            sizeBytes: 2_580_000_000,
            minRamGb: 8
        ),
        ModelInfo(
            id: "gemma4-e4b",
            displayName: "Gemma 4 E4B — 3.6GB",
            repoId: "google/gemma-4-E4B-it",
            url: "https://huggingface.co/google/gemma-4-E4B-it",
            fileName: "model.mlpackage",
            sizeBytes: 3_650_000_000,
            minRamGb: 10
        )
    ]
    
    public static func getModelById(_ modelId: String) -> ModelInfo? {
        return AVAILABLE_MODELS.first { $0.id == modelId }
    }
    
    public static func getDeviceRamGb() -> Int {
        let memory = ProcessInfo.processInfo.physicalMemory
        return Int(memory / (1024 * 1024 * 1024))
    }
    
    public static func recommendedModel() -> ModelInfo {
        let totalRamGb = getDeviceRamGb()
        if totalRamGb >= 12 {
            return getModelById("gemma4-e4b") ?? AVAILABLE_MODELS[0]
        } else {
            return getModelById("gemma4-e2b") ?? AVAILABLE_MODELS[0]
        }
    }
    
    public static func bestSupportedModel() -> ModelInfo? {
        let deviceRamGb = getDeviceRamGb()
        return AVAILABLE_MODELS
            .filter { $0.minRamGb <= deviceRamGb }
            .max { $0.minRamGb < $1.minRamGb }
    }
    
    public static func getModelDir() -> URL {
        let paths = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)
        let modelsDir = paths[0].appendingPathComponent("models")
        if !FileManager.default.fileExists(atPath: modelsDir.path) {
            try? FileManager.default.createDirectory(at: modelsDir, withIntermediateDirectories: true)
        }
        return modelsDir
    }
    
    public static func getModelPath(model: ModelInfo) -> String? {
        // Hub.snapshot downloads the whole repo (or matched files) to a specific directory.
        // We need to find where Hub stores it.
        // By default, Hub uses a cache directory.
        // However, we want to control the location if possible or just use Hub's default.
        
        // Actually, swift-transformers Hub usually uses ~/Library/Caches/huggingface/
        // But we might want to move it to Documents for persistence if needed, 
        // though Caches is also persistent on iOS unless disk is low.
        
        // Let's check if the model exists in Hub's cache.
        // Hub.snapshot(from: repo) returns the URL.
        
        // For now, let's assume we use the default Hub cache.
        // We can't easily check without calling Hub or knowing its internal structure.
        
        // Alternative: we check our own modelsDir if we moved it there.
        let localDir = getModelDir().appendingPathComponent(model.id)
        let modelFile = localDir.appendingPathComponent(model.fileName)
        if FileManager.default.fileExists(atPath: modelFile.path) {
            return modelFile.path
        }
        return nil
    }
    
    public static func isModelDownloaded(model: ModelInfo) -> Bool {
        return getModelPath(model: model) != nil
    }
    
    public static func deleteModel(model: ModelInfo) -> Bool {
        let localDir = getModelDir().appendingPathComponent(model.id)
        do {
            if FileManager.default.fileExists(atPath: localDir.path) {
                try FileManager.default.removeItem(at: localDir)
            }
            return true
        } catch {
            logger.error("Failed to delete model \(model.id): \(error.localizedDescription)")
            return false
        }
    }
    
    public static func downloadModel(
        model: ModelInfo,
        onProgress: @escaping (Double) -> Void,
        onComplete: @escaping (String) -> Void,
        onError: @escaping (String) -> Void
    ) {
        Task {
            do {
                let repo = Hub.Repo(id: model.repoId)
                // We match common CoreML files and config
                let modelDirectory = try await Hub.snapshot(
                    from: repo,
                    progressHandler: { progress in
                        onProgress(progress.fractionCompleted)
                    }
                )
                
                // Move or link to our modelsDir for easier management?
                // Or just use the modelDirectory returned.
                let targetDir = getModelDir().appendingPathComponent(model.id)
                if FileManager.default.fileExists(atPath: targetDir.path) {
                    try? FileManager.default.removeItem(at: targetDir)
                }
                try FileManager.default.createDirectory(at: targetDir.deletingLastPathComponent(), withIntermediateDirectories: true)
                
                // Hub.snapshot returns a URL in the cache. 
                // We can symlink or copy. Copying is safer for persistence.
                try FileManager.default.copyItem(at: modelDirectory, to: targetDir)
                
                onComplete(targetDir.appendingPathComponent(model.fileName).path)
            } catch {
                logger.error("Download failed for \(model.id): \(error.localizedDescription)")
                onError(error.localizedDescription)
            }
        }
    }
}
