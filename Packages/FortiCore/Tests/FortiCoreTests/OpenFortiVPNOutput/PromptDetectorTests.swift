import FortiCore
import Testing

@Suite struct PromptDetectorTests {
    typealias Prompt = PromptDetector.Prompt

    /// Prompt text exactly as openfortivpn prints it, and what must be reported.
    static let prompts: [(text: String, prompt: Prompt)] = [
        ("VPN account password: ", .passwordPrompt),  // main.c:733
        ("Two-factor authentication token: ", .otpPrompt(.tokenInfo)),  // http.c:781
        ("Please enter one-time password: ", .otpPrompt(.htmlForm)),  // http.c:518
    ]

    private func detect(_ chunks: [String]) -> [Prompt] {
        var detector = PromptDetector()
        return chunks.flatMap { detector.append(Array($0.utf8)) }
    }

    @Test func exposesThePromptTextsFromTheSource() {
        #expect(PromptDetector.passwordPromptText == "VPN account password: ")
        #expect(PromptDetector.tokenInfoPromptText == "Two-factor authentication token: ")
        #expect(PromptDetector.htmlFormDefaultPromptText == "Please enter one-time password: ")
        #expect(PromptDetector.htmlFormPromptLead == "Please")
    }

    @Test(arguments: 0..<3)
    func reportsAnUnterminatedPromptImmediately(index: Int) {
        let (text, prompt) = Self.prompts[index]
        #expect(detect([text]) == [prompt])
    }

    @Test(arguments: 0..<3)
    func reportsAPromptSplitAtAnyOffset(index: Int) {
        let (text, prompt) = Self.prompts[index]
        let bytes = Array(text.utf8)
        for cut in 0...bytes.count {
            var detector = PromptDetector()
            let first = detector.append(bytes[..<cut])
            let second = detector.append(bytes[cut...])
            #expect(first + second == [prompt], "cut at \(cut)")
        }
    }

    @Test(arguments: 0..<3)
    func reportsAPromptFedByteByByte(index: Int) {
        let (text, prompt) = Self.prompts[index]
        var detector = PromptDetector()
        var reports: [Prompt] = []
        for byte in text.utf8 {
            reports += detector.append([byte])
        }
        #expect(reports == [prompt])
    }

    @Test func reportsEachOccurrenceOnceAndResetsAtNewline() {
        var detector = PromptDetector()
        #expect(detector.append(Array("VPN account password: ".utf8)) == [.passwordPrompt])
        #expect(detector.append([UInt8]()).isEmpty)
        // The "\n" printed after reading stdin arrives glued to the next log line.
        #expect(detector.append(Array("\nINFO:   Connected to gateway.\n".utf8)).isEmpty)
        #expect(detector.append(Array("Two-factor authentication token: ".utf8)) == [.otpPrompt(.tokenInfo)])
        #expect(detector.append(Array("\nINFO:   Authenticated.\n".utf8)).isEmpty)
        #expect(detector.append(Array("VPN account password: ".utf8)) == [.passwordPrompt])
    }

    @Test func reportsTheHTMLFormPromptOncePerLine() {
        var detector = PromptDetector()
        #expect(detector.append(Array("Please".utf8)) == [.otpPrompt(.htmlForm)])
        #expect(detector.append(Array(" enter one-time password: ".utf8)).isEmpty)
        #expect(detector.append(Array("Please".utf8)).isEmpty)
        #expect(detector.append(Array("\nPlease enter".utf8)) == [.otpPrompt(.htmlForm)])
    }

    /// try_otp_auth copies server text from "Please" up to the next '<' (http.c:500-516); it does
    /// not have to end with ": " and may even contain a newline.
    @Test(arguments: [
        "Please enter your FortiToken code: ",
        "Please enter the verification code sent to your email",
        "Please",
        "Please enter code:\n   ",
        "Please enter one-time password: ",
    ])
    func reportsServerSuppliedHTMLFormPrompts(text: String) {
        #expect(detect([text]) == [.otpPrompt(.htmlForm)])
    }

    @Test func reportsTheSecondOccurrenceOnTheSameLine() {
        #expect(detect(["VPN account password: VPN account password: "]) == [.passwordPrompt, .passwordPrompt])
    }

    @Test func detectsAPromptThatEndsAnUnfinishedLine() {
        #expect(detect(["WARN:   noise without newline; VPN account password: "]) == [.passwordPrompt])
    }

    @Test(arguments: [
        "INFO:   Connected to gateway.\n",
        "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.\n",
        "WARN:   Adding route table is incomplete. Please check route table.\n",
        "ERROR:  Could not authenticate to the gateway. Please make sure tunnel mode is allowed by the gateway, check the realm, etc.\n",
        " Please",
        "please enter one-time password: ",
        "Pleas",
        "VPN account password:",
        "Two-factor authentication token:",
        "add net 0.0.0.0: gateway ppp0\n",
    ])
    func ignoresOtherOutput(text: String) {
        #expect(detect([text]).isEmpty)
    }

    @Test func ignoresAPromptLikeTextBrokenByANewline() {
        #expect(detect(["VPN account pass", "\n", "word: "]).isEmpty)
        #expect(detect(["Ple", "\n", "ase"]).isEmpty)
    }

    @Test func handlesLongLinesWithoutLosingTheTail() {
        let noise = String(repeating: "x", count: 10_000)
        #expect(detect([noise, "VPN account password: "]) == [.passwordPrompt])
        #expect(detect([noise + "\n" + noise + "\n", "Please"]) == [.otpPrompt(.htmlForm)])
    }
}
