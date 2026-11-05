/// Detects openfortivpn's stdin prompts in the raw stdout byte stream, without waiting for a
/// newline or an idle period.
///
/// openfortivpn 1.24.1 prints every prompt through `read_password()` as `printf("%s", prompt);
/// fflush(stdout);` with no newline and then blocks reading stdin (userinput.c:344-345), so the
/// prompt is always the unfinished tail of the current stdout line. Prompts:
/// - `"VPN account password: "` (main.c:732-734) → `.passwordPrompt`
/// - `"Two-factor authentication token: "` (http.c:780-782) → `.otpPrompt`
///
/// Each prompt occurrence is reported once; state resets at every LF. Feed stdout only.
public struct PromptDetector: Sendable {
    public enum Prompt: Equatable, Sendable {
        case passwordPrompt
        case otpPrompt
    }

    static let passwordPromptText = "VPN account password: "
    static let otpPromptText = "Two-factor authentication token: "

    private static let suffixPrompts: [(bytes: [UInt8], prompt: Prompt)] = [
        (Array(passwordPromptText.utf8), .passwordPrompt),
        (Array(otpPromptText.utf8), .otpPrompt),
    ]
    /// Longer than any suffix prompt, so a match can always be checked across chunk boundaries.
    private static let tailCapacity = 64

    /// Last bytes of the current line from earlier `append` calls, bounded by `tailCapacity`.
    private var tail: [UInt8] = []
    /// Bytes of the current line from earlier `append` calls.
    private var lineLength = 0
    /// Line offset where the last suffix prompt ended, so one occurrence is reported once.
    private var lastSuffixMatchEnd = 0

    public init() {}

    /// Feeds stdout bytes as they are read; returns the prompts that completed in them, in order.
    public mutating func append<Bytes: Sequence>(_ bytes: Bytes) -> [Prompt] where Bytes.Element == UInt8 {
        let input = Array(bytes)
        let suffixPrompts = Self.suffixPrompts
        var prompts: [Prompt] = []
        input.withUnsafeBufferPointer { buffer in
            var segmentStart = 0
            for index in buffer.indices {
                let byte = buffer[index]
                if byte == 0x0A {
                    resetLine()
                    segmentStart = index + 1
                    continue
                }
                // Every suffix prompt ends with a space; skip the comparison otherwise.
                guard byte == 0x20 else { continue }
                let lineEnd = lineLength + (index - segmentStart) + 1
                for candidate in suffixPrompts where lineEnd - lastSuffixMatchEnd >= candidate.bytes.count
                    && suffix(candidate.bytes, endsAt: index, segmentStart: segmentStart, in: buffer) {
                    lastSuffixMatchEnd = lineEnd
                    prompts.append(candidate.prompt)
                }
            }
            // Carry the unfinished line over to the next call.
            let rest = buffer[segmentStart...]
            lineLength += rest.count
            tail.append(contentsOf: rest.suffix(Self.tailCapacity))
            if tail.count > Self.tailCapacity {
                tail.removeFirst(tail.count - Self.tailCapacity)
            }
        }
        return prompts
    }

    private mutating func resetLine() {
        tail.removeAll(keepingCapacity: true)
        lineLength = 0
        lastSuffixMatchEnd = 0
    }

    /// Whether the current line, whose last byte is `buffer[index]`, ends with `suffix`. Bytes
    /// before `segmentStart` come from `tail`.
    private func suffix(
        _ suffix: [UInt8],
        endsAt index: Int,
        segmentStart: Int,
        in buffer: UnsafeBufferPointer<UInt8>
    ) -> Bool {
        let inBuffer = index - segmentStart + 1
        for offset in 0..<suffix.count {
            let actual: UInt8
            if offset < inBuffer {
                actual = buffer[index - offset]
            } else {
                let tailIndex = tail.count - 1 - (offset - inBuffer)
                guard tailIndex >= 0 else { return false }
                actual = tail[tailIndex]
            }
            if actual != suffix[suffix.count - 1 - offset] {
                return false
            }
        }
        return true
    }
}
