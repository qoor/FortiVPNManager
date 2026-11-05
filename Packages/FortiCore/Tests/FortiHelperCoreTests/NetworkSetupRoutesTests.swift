import FortiShared
import Testing
@testable import FortiHelperCore

@Suite struct NetworkSetupRoutesTests {
    @Test func noRoutes() throws {
        #expect(try NetworkSetupRoutes.parse("There are no additional IPv4 routes on Ethernet.\n").isEmpty)
        #expect(try NetworkSetupRoutes.parse("").isEmpty)
    }

    @Test func oneRoutePerLine() throws {
        let output = """
            10.30.161.16 255.255.255.255 10.20.161.254
            10.30.170.0  255.255.255.0   10.20.161.254

            """
        #expect(try NetworkSetupRoutes.parse(output) == [
            AdditionalRoute(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254"),
            AdditionalRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254"),
        ])
    }

    @Test func errorsAndUnknownLinesAreFailures() {
        #expect(throws: NetworkSetupRoutes.ParseFailure.reportedError("** Error: The parameters were not valid.")) {
            try NetworkSetupRoutes.parse("** Error: The parameters were not valid.\n")
        }
        #expect(throws: NetworkSetupRoutes.ParseFailure.unexpectedLine("Ethernet is not a recognized network service.")) {
            try NetworkSetupRoutes.parse("Ethernet is not a recognized network service.\n")
        }
        #expect(throws: NetworkSetupRoutes.ParseFailure.unexpectedLine("10.0.0.1 255.255.255.255")) {
            try NetworkSetupRoutes.parse("10.0.0.1 255.255.255.255\n")
        }
    }

    @Test func setArguments() {
        let routes = [
            AdditionalRoute(destination: "10.30.161.16", subnetMask: "255.255.255.255", gateway: "10.20.161.254"),
            AdditionalRoute(destination: "10.30.170.0", subnetMask: "255.255.255.0", gateway: "10.20.161.254"),
        ]
        #expect(NetworkSetupRoutes.setArguments(service: "USB 10/100/1G/2.5G LAN", routes: routes) == [
            "-setadditionalroutes", "USB 10/100/1G/2.5G LAN",
            "10.30.161.16", "255.255.255.255", "10.20.161.254",
            "10.30.170.0", "255.255.255.0", "10.20.161.254",
        ])
        #expect(NetworkSetupRoutes.setArguments(service: "Ethernet", routes: []) == ["-setadditionalroutes", "Ethernet"])
    }

    @Test func reportedError() {
        #expect(NetworkSetupRoutes.reportedError(in: "") == nil)
        #expect(NetworkSetupRoutes.reportedError(in: "  ** Error: denied\n") == "** Error: denied")
    }
}
