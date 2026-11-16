import FVMHelperEntry

/// The Helper entry (Rust) in this process.
nonisolated enum HelperEntry {
    /// Starts the Helper entry and returns the stream to it. `onMessage` runs on the stream's reader
    /// thread.
    static func start(
        onMessage: @escaping @Sendable (HelperInternal.FromEntry) -> Void
    ) -> MessageStream<HelperInternal.FromEntry, HelperInternal.ToEntry> {
        MessageStream(fileDescriptor: fvm_helper_entry_start(), onMessage: onMessage)
    }
}
