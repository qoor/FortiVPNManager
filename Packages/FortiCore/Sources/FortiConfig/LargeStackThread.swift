import Foundation

/// Runs a closure synchronously on a short-lived thread with a large stack.
///
/// Used for YAML composition, whose recursion depth is bounded only by libyaml's nesting cap
/// (1000 levels ≈ 1.3 MiB of stack in release builds), more than a secondary thread's default 512 KiB.
enum LargeStackThread {
    static let stackSize = 16 << 20

    static func run<T: Sendable>(_ body: @escaping @Sendable () -> T) -> T {
        let handoff = Handoff<T>()
        let thread = Thread {
            handoff.put(body())
        }
        thread.name = "FortiConfig.LargeStackThread"
        thread.stackSize = stackSize
        thread.start()
        return handoff.take()
    }

    /// One-shot value hand-off between the worker and the waiting caller.
    private final class Handoff<T: Sendable>: @unchecked Sendable {
        private let condition = NSCondition()
        private var value: T?

        func put(_ newValue: T) {
            condition.lock()
            value = newValue
            condition.signal()
            condition.unlock()
        }

        func take() -> T {
            condition.lock()
            defer { condition.unlock() }
            while value == nil {
                condition.wait()
            }
            return value!
        }
    }
}
