import Foundation
import Transformers
import os.log

/**
 * InferenceSession manages a single generation task.
 * It handles the auto-regressive loop and streams results in a format
 * compatible with the Pokeclaw Android implementation.
 */
public class InferenceSession {
    private static let logger = Logger(subsystem: "io.agents.pokeclaw", category: "InferenceSession")
    
    public let id: String
    private let engine: CoreMLEngine
    private var isRunning: Bool = false
    
    public init(id: String, engine: CoreMLEngine) {
        self.id = id
        self.engine = engine
    }
    
    /**
     * Starts the generation loop.
     * 
     * @param prompt The input text prompt.
     * @param maxNewTokens Maximum number of tokens to generate.
     * @param onEvent Callback for streaming events (token_batch).
     * @param onComplete Callback when generation finishes normally.
     * @param onError Callback when an error occurs.
     */
    public func generate(
        prompt: String,
        maxNewTokens: Int = 512,
        onEvent: @escaping (String, [String: Any]) -> Void,
        onComplete: @escaping () -> Void,
        onError: @escaping (String) -> Void
    ) {
        guard !isRunning else {
            onError("Session is already running")
            return
        }
        
        isRunning = true
        
        Task {
            do {
                let inputIds = try engine.tokenizer.encode(text: prompt)
                var currentIds = inputIds
                var batchIndex = 0
                
                Self.logger.info("Starting generation for session \(self.id)")
                
                // Simple auto-regressive loop. 
                // Note: Real implementation should use more sophisticated sampling (Top-P, Temperature).
                var tokensGenerated = 0
                while tokensGenerated < maxNewTokens && isRunning {
                    // predict returns the model's output for the current sequence
                    let output = try await engine.model.predict(inputIds: currentIds)
                    
                    // Simple greedy sampling: get the token with highest probability
                    // We assume output.logits contains the logits for the last token or 
                    // is a flat array where we can find the max.
                    let nextTokenId = argmax(output.logits)
                    
                    if nextTokenId == engine.tokenizer.eosTokenId {
                        Self.logger.info("EOS token reached for session \(self.id)")
                        break
                    }
                    
                    currentIds.append(nextTokenId)
                    let tokenText = engine.tokenizer.decode(tokens: [nextTokenId])
                    
                    // Match Android contract: {event: "token_batch", data: {tokens, batch_index}}
                    let eventData: [String: Any] = [
                        "tokens": tokenText,
                        "batch_index": batchIndex
                    ]
                    
                    onEvent("token_batch", eventData)
                    
                    batchIndex += 1
                    tokensGenerated += 1
                }
                
                isRunning = false
                onComplete()
            } catch {
                Self.logger.error("Generation failed for session \(self.id): \(error.localizedDescription)")
                isRunning = false
                onError(error.localizedDescription)
            }
        }
    }
    
    /**
     * Stops the generation loop prematurely.
     */
    public func stop() {
        isRunning = false
        Self.logger.info("Session \(self.id) stopped by user")
    }
    
    /**
     * Finds the index of the maximum value in the logits array.
     * This implements simple greedy sampling.
     */
    private func argmax(_ logits: [Float]) -> Int {
        // If logits are for multiple tokens, we'd need to slice for the last one.
        // For simplicity in this scaffold, we assume the engine/model returns 
        // what we need or the last element is representative.
        return logits.enumerated().max(by: { $0.element < $1.element })?.offset ?? 0
    }
}
