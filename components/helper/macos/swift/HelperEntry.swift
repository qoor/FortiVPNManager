import FVMHelperEntry

/// The Helper entry (Rust) in this process.
nonisolated enum HelperEntry {
    /// Starts the Helper entry and returns the stream to it. `onMessage` runs on the stream's reader
    /// thread.
    static func start(
        onMessage: @escaping @Sendable (HelperInternal.FromEntry) -> Void
    ) -> MessageStream<HelperInternal.FromEntry, HelperInternal.ToEntry> {
        let fileDescriptor = fvm_helper_entry_start { text, length in
            let line = String(decoding: UnsafeBufferPointer(start: text, count: length), as: UTF8.self)
            HelperLog.helper.fault("\(line, privacy: .public)")
        }
        return MessageStream(fileDescriptor: fileDescriptor, onMessage: onMessage)
    }
}
