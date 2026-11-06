import Foundation

/// Swift end of the socketpair to Rust. Each message is a 4-byte big-endian length followed by that
/// many bytes of UTF-8 JSON (components/contract/src/stream.rs).
///
/// One thread reads and one serial queue writes, so neither touches the caller's thread. Neither
/// side closes its end on purpose, so a failed read or write, a message that does not decode and the
/// end of the stream all stop the process.
nonisolated final class MessageStream<Incoming: Decodable & Sendable, Outgoing: Encodable & Sendable>: Sendable {
    private let handle: FileHandle
    private let writeQueue = DispatchQueue(label: "com.qoor.FortiVPNManager.message-stream.write")

    /// Takes ownership of `fileDescriptor` and starts reading. `onMessage` runs on the reader thread
    /// for each message, one at a time in arrival order.
    init(fileDescriptor: Int32, onMessage: @escaping @Sendable (Incoming) -> Void) {
        handle = FileHandle(fileDescriptor: fileDescriptor, closeOnDealloc: true)
        let reader = Thread { [handle] in
            let decoder = JSONDecoder()
            while true {
                let payload = Self.readMessage(from: handle)
                let message: Incoming
                do {
                    message = try decoder.decode(Incoming.self, from: payload)
                } catch {
                    fatalError("cannot decode a message from Rust: \(error)")
                }
                onMessage(message)
            }
        }
        reader.name = "com.qoor.FortiVPNManager.message-stream.read"
        reader.start()
    }

    /// Writes happen in call order.
    func send(_ message: Outgoing) {
        // Encoding fails only for non-finite numbers, which no message carries.
        let payload = try! JSONEncoder().encode(message)
        let length = UInt32(payload.count)
        let frame = Data([24, 16, 8, 0].map { UInt8(truncatingIfNeeded: length >> $0) }) + payload
        writeQueue.async { [handle] in
            do {
                try handle.write(contentsOf: frame)
            } catch {
                fatalError("cannot write a message to Rust: \(error)")
            }
        }
    }

    /// Blocks until a whole message arrives.
    private static func readMessage(from handle: FileHandle) -> Data {
        let header = read(4, from: handle)
        let length = header.reduce(0) { $0 << 8 | Int($1) }
        return read(length, from: handle)
    }

    /// Exactly `count` bytes. Never asks for zero bytes, which `read(upToCount:)` answers like the
    /// end of the stream.
    private static func read(_ count: Int, from handle: FileHandle) -> Data {
        var data = Data()
        while data.count < count {
            let chunk: Data?
            do {
                chunk = try handle.read(upToCount: count - data.count)
            } catch {
                fatalError("cannot read a message from Rust: \(error)")
            }
            guard let chunk, !chunk.isEmpty else {
                fatalError("Rust closed its end of the message stream")
            }
            data.append(chunk)
        }
        return data
    }
}
