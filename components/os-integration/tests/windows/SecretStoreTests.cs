using Xunit;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration.Tests;

/// <summary>
/// The credential names, which must match the macOS Keychain items, and a credential written, read
/// and deleted in the Credential Manager of the user running the tests.
/// </summary>
public class SecretStoreTests
{
    [Fact]
    public void CredentialsAreNamedAfterTheKeychainItems()
    {
        SecretItem[] items =
        [
            new SecretItem.VpnPassword("alice", "vpn.example.com"),
            new SecretItem.GmailAppPassword("Alice@example.com"),
            new SecretItem.GoogleRefreshToken("Alice@Example.com"),
            new SecretItem.GoogleClientSecret("987-xyz.apps.googleusercontent.com"),
        ];
        Assert.Equal(
            [
                "com.qoor.FortiVPNManager.vpn/alice@vpn.example.com",
                "com.qoor.FortiVPNManager.gmail/Alice@example.com",
                "com.qoor.FortiVPNManager.google/refresh-token:alice@example.com",
                "com.qoor.FortiVPNManager.google/client-secret:987-xyz.apps.googleusercontent.com",
            ],
            items.Select(SecretStore.TargetName));
    }

    /// <summary>
    /// Credential Manager is open only in a session the user signed in to, as the app's is: in an
    /// SSH session writing fails. So this runs apart, in the user's session.
    /// </summary>
    [Fact]
    [Trait("Needs", "SignedInSession")]
    public void ASecretIsWrittenReadReplacedAndDeleted()
    {
        var item = new SecretItem.GmailAppPassword($"fvm-test-{Guid.NewGuid():N}@example.invalid");
        try
        {
            Assert.Equal(new PortResponse.HasSecret(false), SecretStore.Exists(item));
            Assert.Equal(new PortResponse.Secret(), SecretStore.Read(item));
            // Not ASCII, so the bytes must be UTF-8 both ways.
            Assert.Equal(new PortResponse.SecretChanged(), SecretStore.Write("p@ss-éß", item));
            Assert.Equal(new PortResponse.Secret("p@ss-éß"), SecretStore.Read(item));
            Assert.Equal(new PortResponse.HasSecret(true), SecretStore.Exists(item));
            Assert.Equal(new PortResponse.SecretChanged(), SecretStore.Write("second", item));
            Assert.Equal(new PortResponse.Secret("second"), SecretStore.Read(item));
        }
        finally
        {
            Assert.Equal(new PortResponse.SecretChanged(), SecretStore.Delete(item));
        }
        Assert.Equal(new PortResponse.HasSecret(false), SecretStore.Exists(item));
        // Deleting a missing item succeeds.
        Assert.Equal(new PortResponse.SecretChanged(), SecretStore.Delete(item));
    }
}
