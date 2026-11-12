import Foundation
import Testing

struct ServiceRegistrationTests {
    @Test func theAppInstalledInApplicationsMayRegisterServices() {
        let bundleURL = URL(filePath: "/Applications/FortiVPNManager.app", directoryHint: .isDirectory)
        #expect(ServiceRegistration.isInstalledCopy(at: bundleURL))
    }

    @Test(arguments: [
        "/tmp/FortiVPNManager.app",
        "/Applications/FortiVPNManager Copy.app",
        "/Applications/FortiVPNManager.app/Contents",
    ])
    func otherCopiesMayNotRegisterServices(path: String) {
        let bundleURL = URL(filePath: path, directoryHint: .isDirectory)
        #expect(!ServiceRegistration.isInstalledCopy(at: bundleURL))
    }
}
