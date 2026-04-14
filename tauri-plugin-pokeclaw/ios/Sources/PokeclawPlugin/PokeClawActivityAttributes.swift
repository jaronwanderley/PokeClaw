import ActivityKit
import Foundation

/// ActivityAttributes model for PokeClaw Dynamic Island Live Activities.
/// Static attributes (taskTitle) are set at activity creation and remain constant.
/// ContentState holds dynamic data updated throughout the task lifecycle.
@available(iOS 16.1, *)
public struct PokeClawActivityAttributes: ActivityAttributes {
    /// Static data set once when the Live Activity starts.
    public let taskTitle: String

    /// Dynamic content state updated as the task progresses.
    public struct ContentState: Codable, Hashable {
        /// Current step number (1-based). 0 means not yet started.
        public let step: Int
        /// Total number of steps in the task.
        public let totalSteps: Int
        /// Human-readable description of the current step.
        public let description: String
        /// Task status: "running", "completed", or "failed".
        public let status: String

        public init(step: Int, totalSteps: Int, description: String, status: String) {
            self.step = step
            self.totalSteps = totalSteps
            self.description = description
            self.status = status
        }
    }

    public init(taskTitle: String) {
        self.taskTitle = taskTitle
    }
}
