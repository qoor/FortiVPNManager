import FortiCore
import FortiShared
import Testing

@Suite struct LineAssemblerTests {
    private let t0 = ContinuousClock.now

    private func at(_ milliseconds: Int) -> ContinuousClock.Instant {
        t0 + .milliseconds(milliseconds)
    }

    private func complete(_ text: String) -> AssembledLine {
        AssembledLine(text: text, isPartial: false)
    }

    private func partial(_ text: String) -> AssembledLine {
        AssembledLine(text: text, isPartial: true)
    }

    // MARK: Splitting

    @Test func splitsOnLFAndStripsOneTrailingCR() {
        var assembler = LineAssembler(source: .stdout)
        let lines = assembler.append(Array("a\nb\r\nc\r\r\n".utf8), now: t0)
        #expect(lines == [complete("a"), complete("b"), complete("c^M")])
    }

    @Test func keepsEmptyLinesThatFollowACompleteLine() {
        var assembler = LineAssembler(source: .stdout)
        #expect(assembler.append(Array("x\n\n\r\n".utf8), now: t0) == [complete("x"), complete(""), complete("")])
    }

    @Test func joinsALineSplitAcrossChunks() {
        var assembler = LineAssembler(source: .stdout)
        #expect(assembler.append(Array("INFO:   Conn".utf8), now: at(0)).isEmpty)
        #expect(assembler.append(Array("ected to gate".utf8), now: at(10)).isEmpty)
        #expect(assembler.append(Array("way.\nINFO:".utf8), now: at(20)) == [complete("INFO:   Connected to gateway.")])
        #expect(assembler.append(Array("   Authenticated.\n".utf8), now: at(30)) == [complete("INFO:   Authenticated.")])
    }

    @Test func remembersItsSource() {
        #expect(LineAssembler(source: .stderr).source == .stderr)
    }

    // MARK: Idle flush

    @Test func flushesAnUnfinishedLineOnlyAfterTheIdlePeriod() {
        var assembler = LineAssembler(source: .stdout)
        #expect(assembler.append(Array("VPN account password: ".utf8), now: at(0)).isEmpty)
        #expect(assembler.idleFlushDeadline == at(250))
        #expect(assembler.flushIfIdle(now: at(249)).isEmpty)
        #expect(assembler.flushIfIdle(now: at(250)) == [partial("VPN account password: ")])
        #expect(assembler.idleFlushDeadline == nil)
        #expect(assembler.flushIfIdle(now: at(5_000)).isEmpty)
    }

    @Test func honoursACustomIdlePeriod() {
        var assembler = LineAssembler(source: .stdout, idleFlush: .seconds(1))
        _ = assembler.append(Array("tail".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(999)).isEmpty)
        #expect(assembler.flushIfIdle(now: at(1_000)) == [partial("tail")])
    }

    @Test func measuresIdlenessFromTheLastByte() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("ab".utf8), now: at(0))
        _ = assembler.append(Array("c".utf8), now: at(200))
        #expect(assembler.flushIfIdle(now: at(300)).isEmpty)
        // An empty read does not count as activity.
        _ = assembler.append([UInt8](), now: at(400))
        #expect(assembler.idleFlushDeadline == at(450))
        #expect(assembler.flushIfIdle(now: at(450)) == [partial("abc")])
    }

    @Test func swallowsTheLoneLFThatEndsAFlushedPrompt() {
        // userinput.c:367 prints "\n" after reading stdin; it reaches the pipe together with the
        // next log line (log.c:127 fflush).
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("VPN account password: ".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(300)) == [partial("VPN account password: ")])
        #expect(assembler.append(Array("\nINFO:   Connected to gateway.\n".utf8), now: at(900)) == [
            complete("INFO:   Connected to gateway."),
        ])
    }

    @Test func swallowsALoneLFArrivingInItsOwnChunk() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("Two-factor authentication token: ".utf8), now: at(0))
        _ = assembler.flushIfIdle(now: at(250))
        #expect(assembler.append([0x0A], now: at(400)).isEmpty)
        #expect(assembler.append(Array("INFO:   Authenticated.\n".utf8), now: at(401)) == [complete("INFO:   Authenticated.")])
    }

    @Test func swallowsALoneCRLFAfterAFlushedPartialLine() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("prompt: ".utf8), now: at(0))
        _ = assembler.flushIfIdle(now: at(250))
        #expect(assembler.append(Array("\r\n".utf8), now: at(300)).isEmpty)
    }

    @Test func keepsTheEmptyLineAfterAPromptThatWasNotFlushed() {
        // Answered within the idle period: the prompt line simply completes.
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("VPN account password: ".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(100)).isEmpty)
        #expect(assembler.append(Array("\nINFO:   Connected to gateway.\n\n".utf8), now: at(120)) == [
            complete("VPN account password: "),
            complete("INFO:   Connected to gateway."),
            complete(""),
        ])
    }

    @Test func emitsTextContinuingAfterAPartialFlushAsItsOwnLine() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("abc".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(250)) == [partial("abc")])
        #expect(assembler.append(Array("def\nghi\n".utf8), now: at(260)) == [complete("def"), complete("ghi")])
    }

    @Test func flushesAContinuationAgainWhenItAlsoStalls() {
        var assembler = LineAssembler(source: .stderr)
        _ = assembler.append(Array("Fri Apr  5 11:31:24 2024 : ".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(250)) == [partial("Fri Apr  5 11:31:24 2024 : ")])
        _ = assembler.append(Array("Modem hangup".utf8), now: at(600))
        #expect(assembler.flushIfIdle(now: at(850)) == [partial("Modem hangup")])
        #expect(assembler.append([0x0A], now: at(1_200)).isEmpty)
    }

    @Test func appendFlushesAnOverdueLineBeforeNewBytes() {
        // The result must not depend on whether the caller's idle timer fired in time.
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("Please enter one-time password: ".utf8), now: at(0))
        #expect(assembler.append(Array("\nINFO:   Authenticated.\n".utf8), now: at(20_000)) == [
            partial("Please enter one-time password: "),
            complete("INFO:   Authenticated."),
        ])
    }

    @Test func holdsBackATrailingCROnIdleFlush() {
        var assembler = LineAssembler(source: .stderr)
        _ = assembler.append(Array("abc\r".utf8), now: at(0))
        #expect(assembler.flushIfIdle(now: at(250)) == [partial("abc")])
        // Nothing left to flush, so no deadline that a timer could spin on.
        #expect(assembler.idleFlushDeadline == nil)
        #expect(assembler.flushIfIdle(now: at(500)).isEmpty)
        #expect(assembler.append(Array("\n".utf8), now: at(600)).isEmpty)
    }

    @Test func rendersAHeldBackCRWhenTheLineContinues() {
        var assembler = LineAssembler(source: .stderr)
        _ = assembler.append(Array("50%\r".utf8), now: at(0))
        _ = assembler.flushIfIdle(now: at(250))
        #expect(assembler.append(Array("100%\n".utf8), now: at(300)) == [complete("^M100%")])
    }

    @Test func holdsBackAnIncompleteUTF8SequenceOnIdleFlush() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append([0x61, 0x62, 0xE2, 0x82], now: at(0))  // "ab" + 2 of 3 bytes of "€"
        #expect(assembler.idleFlushDeadline == at(250))
        #expect(assembler.flushIfIdle(now: at(250)) == [partial("ab")])
        #expect(assembler.idleFlushDeadline == nil)
        #expect(assembler.append([0xAC, 0x0A], now: at(300)) == [complete("€")])
    }

    @Test func waitsWhenOnlyAnIncompleteSequenceIsPending() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append([0xF0, 0x9F], now: at(0))
        #expect(assembler.idleFlushDeadline == nil)
        #expect(assembler.flushIfIdle(now: at(1_000)).isEmpty)
        #expect(assembler.finish() == [partial("\u{FFFD}")])
    }

    // MARK: Hard split

    @Test func hardSplitsLinesLongerThanTheLimit() {
        var assembler = LineAssembler(source: .stdout, maxLineBytes: 8)
        #expect(assembler.append(Array("0123456789ABCDEFGHIJ\n".utf8), now: t0) == [
            partial("01234567"),
            partial("89ABCDEF"),
            complete("GHIJ"),
        ])
    }

    @Test func doesNotSplitALineOfExactlyTheLimit() {
        var assembler = LineAssembler(source: .stdout, maxLineBytes: 8)
        #expect(assembler.append(Array("01234567\n".utf8), now: t0) == [complete("01234567")])
        #expect(assembler.append(Array("01234567\r\n".utf8), now: t0) == [complete("01234567")])
        #expect(assembler.append(Array("01234567\rX\n".utf8), now: t0) == [partial("01234567"), complete("^MX")])
    }

    @Test func swallowsTheLFAfterAHardSplitThatConsumedTheWholeLine() {
        var assembler = LineAssembler(source: .stdout, maxLineBytes: 4)
        #expect(assembler.append(Array("01234567".utf8), now: t0) == [partial("0123")])
        #expect(assembler.append(Array("8\n".utf8), now: t0) == [partial("4567"), complete("8")])
        #expect(assembler.append(Array("abcde".utf8), now: t0) == [partial("abcd")])
        #expect(assembler.flushIfIdle(now: at(300)) == [partial("e")])
        #expect(assembler.append(Array("\n".utf8), now: at(301)).isEmpty)
    }

    @Test func usesA16KiBLimitByDefault() {
        var assembler = LineAssembler(source: .stdout)
        let lines = assembler.append(Array(repeating: UInt8(ascii: "x"), count: 40_000) + [0x0A], now: t0)
        #expect(lines.map(\.text.utf8.count) == [16_384, 16_384, 7_232])
        #expect(lines.map(\.isPartial) == [true, true, false])
    }

    @Test(arguments: 4...13)
    func hardSplitKeepsUTF8ScalarsIntact(maxLineBytes: Int) {
        let text = "a\u{AC00}\u{B098}😀b€\u{B2E4}cd\u{B77C}\u{B9C8}😀😀\u{BC14}"
        var assembler = LineAssembler(source: .stdout, maxLineBytes: maxLineBytes)
        var lines: [AssembledLine] = []
        for byte in Array(text.utf8) + [0x0A] {
            lines += assembler.append([byte], now: t0)
        }
        #expect(lines.map(\.text).joined() == text)
        #expect(lines.allSatisfy { !$0.text.contains("\u{FFFD}") && $0.text.utf8.count <= maxLineBytes })
        #expect(lines.dropLast().allSatisfy { $0.isPartial })
        #expect(lines.last?.isPartial == false)
    }

    // MARK: Rendering

    @Test(arguments: [
        ([0x61, 0xFF, 0x62], "a\u{FFFD}b"),
        ([0xC3, 0x28], "\u{FFFD}("),
        ([0xED, 0xA0, 0x80], "\u{FFFD}\u{FFFD}\u{FFFD}"),  // UTF-16 surrogate encoded in UTF-8
        (Array("\u{D55C}\u{AE00} OK".utf8), "\u{D55C}\u{AE00} OK"),
    ] as [([UInt8], String)])
    func replacesInvalidUTF8(bytes: [UInt8], expected: String) {
        var assembler = LineAssembler(source: .stdout)
        #expect(assembler.append(bytes + [0x0A], now: t0) == [complete(expected)])
    }

    @Test(arguments: [
        ("\u{1B}[0;31mERROR:  x\u{1B}[0;0m", "^[[0;31mERROR:  x^[[0;0m"),
        ("a\u{0}b", "a^@b"),
        ("del\u{7F}", "del^?"),
        ("bell\u{7}", "bell^G"),
        ("us\u{1F}", "us^_"),
        ("mid\rcr", "mid^Mcr"),
        ("tab\tkept", "tab\tkept"),
        ("nel\u{85}c1\u{9B}", "nelM-^Ec1M-^["),
        ("plain text: é ü 中", "plain text: é ü 中"),
    ])
    func rendersControlCharactersInCaretNotation(raw: String, expected: String) {
        var assembler = LineAssembler(source: .stdout)
        #expect(assembler.append(Array(raw.utf8) + [0x0A], now: t0) == [complete(expected)])
    }

    // MARK: End of stream

    @Test func finishEmitsThePendingLineAsPartial() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("done\nINFO:   Logged".utf8), now: t0)
        #expect(assembler.finish() == [partial("INFO:   Logged")])
        #expect(assembler.finish().isEmpty)
    }

    @Test func finishDropsATrailingCRAndEmitsNothingWhenEmpty() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("abc\r".utf8), now: t0)
        #expect(assembler.finish() == [partial("abc")])
        _ = assembler.append(Array("\r".utf8), now: t0)
        #expect(assembler.finish().isEmpty)
    }

    @Test func finishAfterAPartialFlushEmitsNothingMore() {
        var assembler = LineAssembler(source: .stdout)
        _ = assembler.append(Array("Two-factor authentication token: ".utf8), now: at(0))
        _ = assembler.flushIfIdle(now: at(250))
        #expect(assembler.finish().isEmpty)
        #expect(assembler.idleFlushDeadline == nil)
    }
}
