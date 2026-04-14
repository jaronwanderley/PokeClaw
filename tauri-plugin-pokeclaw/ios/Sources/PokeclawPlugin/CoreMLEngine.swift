import Foundation
import Transformers
import CoreML
import os.log

/**
 * CoreMLEngine manages the loading and configuration of Core ML models
 * for LLM inference on iOS.
 */
public class CoreMLEngine {
    private static let logger = Logger(subsystem: "io.agents.pokeclaw", category: "CoreMLEngine")
    
    public let modelInfo: ModelManager.ModelInfo
    public let tokenizer: Tokenizer
    public let model: LanguageModel
    
    public enum ComputeUnit: String {
        case all = "all"
        case cpuAndGPU = "cpuAndGPU"
        case cpuAndNeuralEngine = "cpuAndNeuralEngine"
        case cpuOnly = "cpuOnly"
        
        var mlComputeUnits: MLComputeUnits {
            switch self {
            case .all: return .all
            case .cpuAndGPU: return .cpuAndGPU
            case .cpuAndNeuralEngine: return .cpuAndNeuralEngine
            case .cpuOnly: return .cpuOnly
            }
        }
    }
    
    public init(modelInfo: ModelManager.ModelInfo, computeUnit: ComputeUnit = .all) async throws {
        self.modelInfo = modelInfo
        
        guard let modelPath = ModelManager.getModelPath(model: modelInfo) else {
            throw NSError(domain: "CoreMLEngine", code: 404, userInfo: [NSLocalizedDescriptionKey: "Model not found locally"])
        }
        
        let modelDir = URL(fileURLWithPath: modelPath).deletingLastPathComponent()
        
        // Load tokenizer using AutoTokenizer from swift-transformers
        Self.logger.info("Loading tokenizer from \(modelDir.path)")
        self.tokenizer = try await AutoTokenizer.from(modelDirectory: modelDir)
        
        // Load model with specific compute units
        let config = MLModelConfiguration()
        config.computeUnits = computeUnit.mlComputeUnits
        
        Self.logger.info("Loading Core ML model from \(modelPath) with units: \(computeUnit.rawValue)")
        // LanguageModel.from is the standard entry point in swift-transformers for CoreML models
        self.model = try await LanguageModel.from(modelDirectory: modelDir, configuration: config)
        
        Self.logger.info("CoreMLEngine initialized successfully")
    }
}
