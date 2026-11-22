import Foundation

/// Code-signing facts used on both ends of the XPC connection to pin the peer.
enum CodeSigningIdentity {
  /// Requirement a peer must satisfy: Apple-issued certificate, same team, given signing identifier.
  static func requirement(identifier: String, teamID: String) -> String {
    "anchor apple generic and identifier \"\(identifier)\" and certificate leaf[subject.OU] = \"\(teamID)\""
  }
}
