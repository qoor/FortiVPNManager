import FortiShared
import Foundation

/// Text side of `networksetup -getadditionalroutes` / `-setadditionalroutes`.
enum NetworkSetupRoutes {
    enum ParseFailure: Error, Sendable, Equatable {
        /// networksetup printed an error (`** Error: …`).
        case reportedError(String)
        /// A line that is neither a route nor the "no routes" notice.
        case unexpectedLine(String)
    }

    /// Parses the output of `-getadditionalroutes <service>`: one `destination mask gateway` line per
    /// route, or `There are no additional IPv4 routes on <service>.` when there are none.
    static func parse(_ output: String) throws(ParseFailure) -> [AdditionalRoute] {
        var routes: [AdditionalRoute] = []
        for rawLine in output.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.isEmpty || line.hasPrefix("There are no additional IPv4 routes") { continue }
            if line.hasPrefix("**") { throw .reportedError(line) }
            let fields = line.split(whereSeparator: \.isWhitespace)
            guard fields.count == 3, fields.allSatisfy({ IPv4Network.parseDottedQuad($0) != nil }) else {
                throw .unexpectedLine(line)
            }
            routes.append(AdditionalRoute(
                destination: String(fields[0]), subnetMask: String(fields[1]), gateway: String(fields[2])))
        }
        return routes
    }

    /// Arguments (after the executable) that replace the service's additional routes with `routes`;
    /// an empty list removes them all.
    static func setArguments(service: String, routes: [AdditionalRoute]) -> [String] {
        ["-setadditionalroutes", service] + routes.flatMap { [$0.destination, $0.subnetMask, $0.gateway] }
    }

    /// networksetup reports most failures on stdout with exit status 0, as `** Error: …` lines.
    static func reportedError(in output: String) -> String? {
        output.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { $0.hasPrefix("**") }
    }
}
