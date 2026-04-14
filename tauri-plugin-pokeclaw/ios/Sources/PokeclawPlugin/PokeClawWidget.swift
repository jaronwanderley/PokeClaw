import SwiftUI
import ActivityKit

// MARK: - Dynamic Island Widget Views
//
// These views define the visual presentation of Live Activities on the
// Dynamic Island (iPhone 14 Pro+) and Lock Screen (all devices).
// They are referenced by the Widget Extension target in Xcode.

@available(iOS 16.1, *)
struct PokeClawLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: PokeClawActivityAttributes.self) { context in
            // Lock Screen / Banner presentation
            lockScreenView(context: context)
        } dynamicIsland: { context in
            DynamicIsland {
                // Expanded region — full banner
                DynamicIslandExpandedRegion(.leading) {
                    expandedLeadingView(context: context)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    expandedTrailingView(context: context)
                }
                DynamicIslandExpandedRegion(.center) {
                    expandedCenterView(context: context)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    expandedBottomView(context: context)
                }
            } compactLeading: {
                compactLeadingView(context: context)
            } compactTrailing: {
                compactTrailingView(context: context)
            } minimal: {
                minimalView(context: context)
            }
        }
    }

    // MARK: - Compact Leading
    // Status indicator icon: green circle (running), checkmark (completed), X (failed)

    @ViewBuilder
    private func compactLeadingView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let status = context.state.status
        if status == "completed" {
            Image(systemName: "checkmark.circle.fill")
                .font(.caption2)
                .foregroundColor(.green)
        } else if status == "failed" {
            Image(systemName: "xmark.circle.fill")
                .font(.caption2)
                .foregroundColor(.red)
        } else {
            Circle()
                .fill(Color.green)
                .frame(width: 8, height: 8)
        }
    }

    // MARK: - Compact Trailing
    // Step count text "X/Y"

    @ViewBuilder
    private func compactTrailingView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let step = context.state.step
        let total = context.state.totalSteps
        if total > 0 {
            Text("\(step)/\(total)")
                .font(.caption2.monospacedDigit())
                .foregroundColor(.white)
        }
    }

    // MARK: - Minimal
    // Single green circle indicator

    @ViewBuilder
    private func minimalView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let status = context.state.status
        if status == "completed" {
            Image(systemName: "checkmark.circle.fill")
                .font(.caption2)
                .foregroundColor(.green)
        } else if status == "failed" {
            Image(systemName: "xmark.circle.fill")
                .font(.caption2)
                .foregroundColor(.red)
        } else {
            Circle()
                .fill(Color.green)
                .frame(width: 8, height: 8)
        }
    }

    // MARK: - Expanded Views
    // Full banner: task title, step progress, description, progress bar

    @ViewBuilder
    private func expandedLeadingView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let status = context.state.status
        if status == "completed" {
            Image(systemName: "checkmark.circle.fill")
                .foregroundColor(.green)
                .font(.title3)
        } else if status == "failed" {
            Image(systemName: "xmark.circle.fill")
                .foregroundColor(.red)
                .font(.title3)
        } else {
            ProgressView()
                .tint(.green)
        }
    }

    @ViewBuilder
    private func expandedTrailingView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let step = context.state.step
        let total = context.state.totalSteps
        if total > 0 {
            Text("Step \(step) of \(total)")
                .font(.caption.monospacedDigit())
                .foregroundColor(.secondary)
        }
    }

    @ViewBuilder
    private func expandedCenterView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        Text(context.attributes.taskTitle)
            .font(.caption)
            .fontWeight(.semibold)
            .foregroundColor(.white)
            .lineLimit(1)
    }

    @ViewBuilder
    private func expandedBottomView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        VStack(spacing: 6) {
            // Description text
            Text(context.state.description)
                .font(.caption2)
                .foregroundColor(.secondary)
                .lineLimit(2)

            // Linear progress bar
            let step = max(context.state.step, 0)
            let total = max(context.state.totalSteps, 1)
            let progress = Double(step) / Double(total)

            ProgressView(value: progress, total: 1.0)
                .tint(context.state.status == "failed" ? .red : .green)
                .progressViewStyle(.linear)
        }
    }

    // MARK: - Lock Screen Presentation

    @ViewBuilder
    private func lockScreenView(context: ActivityViewContext<PokeClawActivityAttributes>) -> some View {
        let step = max(context.state.step, 0)
        let total = max(context.state.totalSteps, 1)
        let progress = Double(step) / Double(total)

        HStack(spacing: 12) {
            // Status icon
            statusIcon(status: context.state.status)
                .font(.title2)

            // Content
            VStack(alignment: .leading, spacing: 4) {
                Text(context.attributes.taskTitle)
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundColor(.white)
                    .lineLimit(1)

                Text(context.state.description)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            // Step counter
            if total > 0 {
                Text("\(step)/\(total)")
                    .font(.caption.monospacedDigit())
                    .foregroundColor(.secondary)
            }
        }
        .padding()
        .background(.black)
    }

    @ViewBuilder
    private func statusIcon(status: String) -> some View {
        if status == "completed" {
            Image(systemName: "checkmark.circle.fill")
                .foregroundColor(.green)
        } else if status == "failed" {
            Image(systemName: "xmark.circle.fill")
                .foregroundColor(.red)
        } else {
            Image(systemName: "circle.fill")
                .foregroundColor(.green)
        }
    }
}

// MARK: - Preview Provider

@available(iOS 16.1, *)
struct PokeClawLiveActivity_Previews: PreviewProvider {
    static let attributes = PokeClawActivityAttributes(taskTitle: "Send WhatsApp Message")

    static let runningState = PokeClawActivityAttributes.ContentState(
        step: 2,
        totalSteps: 5,
        description: "Opening WhatsApp contact list",
        status: "running"
    )

    static let completedState = PokeClawActivityAttributes.ContentState(
        step: 5,
        totalSteps: 5,
        description: "Message sent successfully",
        status: "completed"
    )

    static let failedState = PokeClawActivityAttributes.ContentState(
        step: 3,
        totalSteps: 5,
        description: "Contact not found",
        status: "failed"
    )

    static var previews: some View {
        Group {
            // Lock screen preview
            PokeClawLiveActivity().widgetContentView(
                attributes: attributes,
                state: runningState
            )
            .previewDisplayName("Lock Screen — Running")

            PokeClawLiveActivity().widgetContentView(
                attributes: attributes,
                state: completedState
            )
            .previewDisplayName("Lock Screen — Completed")

            PokeClawLiveActivity().widgetContentView(
                attributes: attributes,
                state: failedState
            )
            .previewDisplayName("Lock Screen — Failed")
        }
    }
}

// Helper to render widget content view in previews without full ActivityKit context.
@available(iOS 16.1, *)
private extension Widget {
    @ViewBuilder
    func widgetContentView(attributes: PokeClawActivityAttributes, state: PokeClawActivityAttributes.ContentState) -> some View {
        // This is a simplified preview helper — the real rendering happens through ActivityConfiguration.
        Text("Preview: \(attributes.taskTitle) — Step \(state.step)/\(state.totalSteps)")
    }
}
