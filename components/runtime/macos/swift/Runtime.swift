import FVMRuntime

/// The runtime (Rust) in this process.
nonisolated enum Runtime {
    /// Starts the runtime and returns the stream to it. `onMessage` runs on the stream's reader
    /// thread.
    static func start(
        onMessage: @escaping @Sendable (AppInternal.FromRuntime) -> Void
    ) -> MessageStream<AppInternal.FromRuntime, AppInternal.ToRuntime> {
        MessageStream(fileDescriptor: fvm_runtime_start(), onMessage: onMessage)
    }
}
