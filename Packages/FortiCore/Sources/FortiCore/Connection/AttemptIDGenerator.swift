import Foundation
import FortiShared

/// Deterministic source of attempt IDs so the reducer stays pure. The app seeds it with a random UUID
/// once per launch; tests use a fixed seed. IDs are `seed` with the low 64 bits XOR-ed by a counter.
public struct AttemptIDGenerator: Sendable, Equatable {
    public let seed: UUID
    public private(set) var issued: UInt64 = 0

    public init(seed: UUID) {
        self.seed = seed
    }

    public mutating func next() -> AttemptID {
        issued += 1
        var bytes = seed.uuid
        let counter = issued
        withUnsafeMutableBytes(of: &bytes) { raw in
            for index in 0..<8 {
                raw[8 + index] ^= UInt8(truncatingIfNeeded: counter >> (8 * (7 - index)))
            }
        }
        return UUID(uuid: bytes)
    }
}
