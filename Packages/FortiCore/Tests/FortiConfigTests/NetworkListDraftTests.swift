import FortiShared
import Testing
@testable import FortiConfig

@Suite struct NetworkListDraftTests {
    let host = bypassHost
    let network = bypassNetwork
    let other = try! IPv4Network(parsing: "10.88.100.10")

    @Test func startsFromTheSavedListWithoutChanges() {
        let draft = NetworkListDraft(saved: [host, network])
        #expect(draft.rows == ["10.77.161.16", "10.77.170.0/24"])
        #expect(!draft.hasChanges)
        #expect(draft.problems.isEmpty)
        #expect(draft.entries == [host, network])
    }

    @Test func editsAreChangesUntilDiscarded() {
        var draft = NetworkListDraft(saved: [host])
        draft.setRows(["10.77.161.16", "  10.77.170.0/24 ", ""])
        #expect(draft.rows == ["10.77.161.16", "10.77.170.0/24"])
        #expect(draft.hasChanges)
        #expect(draft.entries == [host, network])
        draft.discardChanges()
        #expect(draft.rows == ["10.77.161.16"])
        #expect(!draft.hasChanges)
    }

    @Test func respellingIsNoChange() {
        var draft = NetworkListDraft(saved: [host])
        draft.setRows(["10.77.161.16/32"])
        #expect(!draft.hasChanges)
    }

    @Test func problemRowsBlockApply() {
        var draft = NetworkListDraft(saved: [])
        draft.setRows(["10.77.161", "10.77.161.16", "10.77.161.16/32", "10.77.170.5/24"])
        #expect(draft.problems == [
            0: .invalid(.malformed),
            2: .duplicate(host),
            3: .invalid(.hostBitsSet(network: network)),
        ])
        #expect(draft.entries == nil)
        #expect(draft.hasChanges)
        #expect(draft.firstProblemMessage?.contains("10.77.161") == true)
    }

    @Test func overlappingRowsBlockApply() throws {
        var draft = NetworkListDraft(saved: [])
        draft.setRows(["10.77.170.0/24", "10.77.170.9", "10.77.0.0/16"])
        #expect(draft.problems == [1: .overlap(other: network), 2: .overlap(other: network)])
        #expect(draft.entries == nil)
        #expect(draft.firstProblemMessage == "'10.77.170.9' overlaps 10.77.170.0/24, which is already in the list.")
    }

    @Test func duplicateMessage() {
        var draft = NetworkListDraft(saved: [])
        draft.setRows(["10.77.161.16", "10.77.161.16/32"])
        #expect(draft.firstProblemMessage == "10.77.161.16 is already in the list.")
    }

    @Test func withoutEditsTheRowsFollowTheSavedList() {
        var draft = NetworkListDraft(saved: [host])
        draft.merge(saved: [network, host])
        #expect(draft.rows == ["10.77.170.0/24", "10.77.161.16"])
        #expect(!draft.hasChanges)
    }

    @Test func outsideChangesMergeIntoEdits() {
        var draft = NetworkListDraft(saved: [host, network])
        draft.setRows(["10.77.161.16", "10.77.170.0/24", "10.77.99"])
        // Elsewhere: `network` removed, `other` added.
        draft.merge(saved: [host, other])
        #expect(draft.rows == ["10.77.161.16", "10.77.99", "10.88.100.10"])
        #expect(draft.saved == [host, other])
        #expect(draft.hasChanges)
    }

    @Test func addSkipsAnEntryAlreadyListed() {
        var draft = NetworkListDraft(saved: [network])
        draft.add(network)
        draft.add(host)
        #expect(draft.rows == ["10.77.170.0/24", "10.77.161.16"])
    }

    @Test func rowsOverlappingAnEntry() throws {
        var draft = NetworkListDraft(saved: [network])
        draft.setRows(["10.77.170.0/24", "not an address"])
        #expect(draft.hasRow(overlapping: network))
        #expect(draft.hasRow(overlapping: try IPv4Network(parsing: "10.77.170.5")))
        #expect(draft.hasRow(overlapping: try IPv4Network(parsing: "10.77.0.0/16")))
        #expect(!draft.hasRow(overlapping: host))
    }

    @Test func mergeDoesNotRepeatARowTheUserAlreadyTyped() {
        var draft = NetworkListDraft(saved: [host])
        draft.setRows(["10.77.161.16", "10.88.100.10/32"])
        draft.merge(saved: [host, other])
        #expect(draft.rows == ["10.77.161.16", "10.88.100.10/32"])
        #expect(!draft.hasChanges)
    }
}
