import ActivityKit
import Foundation
import os.log

/// Singleton manager for PokeClaw Live Activity lifecycle.
///
/// Provides a simple API to start, update, and stop Live Activities
/// displayed on the Dynamic Island (iPhone 14 Pro+) and Lock Screen.
///
/// All lifecycle events are logged via os_log for observability.
/// ActivityKit activities auto-expire after 12 hours per Apple's limits.
/// Stale activities are cleaned up during `stopActivity()`.
@available(iOS 16.1, *)
public final class LiveActivityManager {

    public static let shared = LiveActivityManager()

    private let logger = Logger(subsystem: "io.agents.pokeclaw", category: "LiveActivity")

    /// The currently active Live Activity, if any.
    private var currentActivity: Activity<PokeClawActivityAttributes>?

    private init() {
        logger.info("LiveActivityManager initialized")
    }

    // MARK: - Start

    /// Start a new Live Activity for the given task.
    ///
    /// Checks `ActivityAuthorizationInfo().areActivitiesEnabled` before starting.
    /// If a Live Activity is already running, it is stopped first.
    ///
    /// - Parameter title: The task title shown in the Dynamic Island.
    /// - Returns: `true` if the activity was started successfully, `false` otherwise.
    @discardableResult
    public func startActivity(title: String) -> Bool {
        // Check authorization
        let authInfo = ActivityAuthorizationInfo()
        guard authInfo.areActivitiesEnabled else {
            logger.warning("startActivity: Live Activities not enabled for this device/app — title=%{public}@", title)
            return false
        }

        // Stop any existing activity before starting a new one
        if currentActivity != nil {
            logger.info("startActivity: stopping existing activity before starting new one")
            stopActivity()
        }

        let attributes = PokeClawActivityAttributes(taskTitle: title)
        let initialState = PokeClawActivityAttributes.ContentState(
            step: 0,
            totalSteps: 0,
            description: "Starting task…",
            status: "running"
        )

        do {
            let activity = try Activity.request(
                attributes: attributes,
                content: .init(state: initialState, staleDate: nil)
            )
            currentActivity = activity
            logger.info("startActivity: started activity id=%{public}@, title=%{public}@", activity.id, title)
            return true
        } catch {
            logger.error("startActivity: failed to start activity — %{public}@", error.localizedDescription)
            return false
        }
    }

    // MARK: - Update

    /// Update the dynamic content of the current Live Activity.
    ///
    /// No-op if no activity is currently running.
    ///
    /// - Parameters:
    ///   - step: Current step number (1-based).
    ///   - totalSteps: Total number of steps.
    ///   - description: Human-readable description of the current step.
    ///   - status: Task status — "running", "completed", or "failed".
    public func updateActivity(step: Int, totalSteps: Int, description: String, status: String) {
        guard let activity = currentActivity else {
            logger.warning("updateActivity: no active Live Activity to update")
            return
        }

        let newState = PokeClawActivityAttributes.ContentState(
            step: step,
            totalSteps: totalSteps,
            description: description,
            status: status
        )

        let content = ActivityContent(
            state: newState,
            staleDate: Calendar.current.date(byAdding: .hour, value: 12, to: Date())
        )

        Task {
            await activity.update(content)
            logger.info("updateActivity: updated activity id=%{public}@, step=%d/%d, status=%{public}@", activity.id, step, totalSteps, status)
        }
    }

    // MARK: - Stop

    /// End the current Live Activity and clean up stale activities.
    ///
    /// Sets the final state to the provided parameters before ending.
    /// Also removes any stale activities that may have accumulated.
    public func stopActivity() {
        guard let activity = currentActivity else {
            logger.debug("stopActivity: no active Live Activity to stop")
            cleanupStaleActivities()
            return
        }

        let finalState = PokeClawActivityAttributes.ContentState(
            step: 0,
            totalSteps: 0,
            description: "Task finished",
            status: "completed"
        )

        let finalContent = ActivityContent(state: finalState, staleDate: Date())

        Task {
            await activity.end(finalContent, dismissalPolicy: .after(.now + 5))
            logger.info("stopActivity: ended activity id=%{public}@", activity.id)
        }

        currentActivity = nil
        cleanupStaleActivities()
    }

    // MARK: - Stale Activity Cleanup

    /// Remove any stale or ended activities that are still in the ActivityKit store.
    ///
    /// Called automatically during `stopActivity()`. Can also be called manually
    /// at app launch to clean up activities left from a previous session.
    public func cleanupStaleActivities() {
        let activities = Activity<PokeClawActivityAttributes>.activities
        for activity in activities {
            if activity.activityState == .ended || activity.activityState == .dismissed {
                Task {
                    await activity.end(nil, dismissalPolicy: .immediate)
                }
                logger.info("cleanupStaleActivities: removed stale activity id=%{public}@, state=%{public}@", activity.id, String(describing: activity.activityState))
            }
        }
    }

    // MARK: - Query

    /// Whether a Live Activity is currently active.
    public var isActivityRunning: Bool {
        return currentActivity != nil && currentActivity?.activityState == .active
    }

    /// The ID of the current activity, if any.
    public var activityId: String? {
        return currentActivity?.id
    }
}
