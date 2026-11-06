import Darwin

enum IPv4 {
    /// True for a dotted-quad address `inet_pton` accepts (never an option-like or shorthand string).
    static func isAddress(_ text: String) -> Bool {
        var address = in_addr()
        return inet_pton(AF_INET, text, &address) == 1
    }
}
