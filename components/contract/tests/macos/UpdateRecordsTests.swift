import Foundation
import Testing

struct UpdateRecordsTests {
    @Test func recordsMatchTheRustConstants() throws {
        let tests = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        let data = try Data(contentsOf: tests.appending(path: "fixtures/app-helper/update_records.json"))
        let records = try JSONDecoder().decode([String: String].self, from: data)
        #expect(records["macosDirectory"] == UpdateRecords.directory)
        #expect(records["startedSuffix"] == UpdateRecords.startedSuffix)
        #expect(records["resultSuffix"] == UpdateRecords.resultSuffix)
    }
}
