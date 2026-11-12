import Foundation
import Testing

struct ServiceRegistrationTests {
    @Test(arguments: [
        "/Applications/FortiVPNManager.app/",
        "/Applications/FortiVPNManager.app",
    ])
    func theAppInstalledInApplicationsMayRegisterServices(path: String) {
        #expect(ServiceRegistration.isInstalledCopy(at: URL(filePath: path)))
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
