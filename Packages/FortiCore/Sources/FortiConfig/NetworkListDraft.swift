import FortiShared
import Foundation

/// One list of the Exception Rules as edited in Settings, `routes.networks` or
/// `routes.bypass_vpn`: rows as typed, against the list as saved. Nothing is saved until Apply writes
/// ``entries``; Discard Changes goes back to ``saved``. Changes made elsewhere (the file, routes added
/// outside the app) are merged into the edits with ``merge(saved:)``.
public struct NetworkListDraft: Sendable, Equatable {
    public enum RowProblem: Sendable, Equatable {
        case invalid(IPv4Network.ParseError)
        /// The row names the same host or network as an earlier row.
        case duplicate(IPv4Network)
        /// The row overlaps the earlier row's `other` (one contains the other).
        case overlap(other: IPv4Network)
    }

    /// Rows as typed, trimmed; never empty.
    public private(set) var rows: [String]
    /// The list the edits are based on.
    public private(set) var saved: [IPv4Network]

    public init(saved: [IPv4Network] = []) {
        self.saved = saved
        rows = saved.map(\.description)
    }

    /// Replaces the rows (from the list editor); blank rows are dropped.
    public mutating func setRows(_ newRows: [String]) {
        rows = newRows.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    }

    /// Appends `entry` unless a row already names it.
    public mutating func add(_ entry: IPv4Network) {
        if !rows.contains(where: { (try? IPv4Network(parsing: $0)) == entry }) {
            rows.append(entry.description)
        }
    }

    /// Whether a row names `entry` or overlaps it (adding it would make a problem row).
    public func hasRow(overlapping entry: IPv4Network) -> Bool {
        rows.contains { (try? IPv4Network(parsing: $0))?.overlaps(entry) == true }
    }

    public mutating func discardChanges() {
        rows = saved.map(\.description)
    }

    /// Problems by row index.
    public var problems: [Int: RowProblem] {
        var result: [Int: RowProblem] = [:]
        var seen: [IPv4Network] = []
        for (index, row) in rows.enumerated() {
            do throws(IPv4Network.ParseError) {
                let entry = try IPv4Network(parsing: row)
                if seen.contains(entry) {
                    result[index] = .duplicate(entry)
                } else if let other = seen.first(where: { $0.overlaps(entry) }) {
                    result[index] = .overlap(other: other)
                } else {
                    seen.append(entry)
                }
            } catch {
                result[index] = .invalid(error)
            }
        }
        return result
    }

    /// The rows as entries, or `nil` while a row has a problem.
    public var entries: [IPv4Network]? {
        var result: [IPv4Network] = []
        for row in rows {
            guard let entry = try? IPv4Network(parsing: row), !result.contains(where: { $0.overlaps(entry) }) else {
                return nil
            }
            result.append(entry)
        }
        return result
    }

    /// Whether Apply would change the list (rows with problems count as changes).
    public var hasChanges: Bool {
        entries != saved
    }

    /// Localized explanation of the first problem row, if any.
    public var firstProblemMessage: String? {
        guard let index = problems.keys.min(), let problem = problems[index] else { return nil }
        switch problem {
        case .invalid(let error):
            return error.message(for: rows[index])
        case .duplicate(let entry):
            return String(localized: "\(entry.description) is already in the list.", bundle: .main)
        case .overlap(let other):
            return String(localized: "'\(ConfigText.preview(rows[index]))' overlaps \(other.description), which is already in the list.", bundle: .main)
        }
    }

    /// The list is now `newSaved` (Apply, a file edit, or routes added outside the app). Without edits
    /// the rows simply follow it; with edits, rows of entries removed there go away, entries added
    /// there are appended, and everything else stays as typed.
    public mutating func merge(saved newSaved: [IPv4Network]) {
        defer { saved = newSaved }
        guard hasChanges else {
            rows = newSaved.map(\.description)
            return
        }
        let removed = saved.filter { !newSaved.contains($0) }
        rows.removeAll { row in
            (try? IPv4Network(parsing: row)).map(removed.contains) ?? false
        }
        for entry in newSaved where !saved.contains(entry) {
            add(entry)
        }
    }
}
