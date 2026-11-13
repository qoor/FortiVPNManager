using System.Security;
using Microsoft.Win32;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The SSL VPN connections of the official FortiClient, which Windows imports its settings from.
/// FortiClient keeps each connection as a key of its own under the machine's software settings, with
/// the server in its Server value (host:port); the key is only read. IPsec connections live elsewhere
/// and are not openconnect's. The username FortiClient shows is kept encrypted by FortiClient, so it
/// is not read.
/// </summary>
internal static class FortiClientSettings
{
    private const string Tunnels = @"SOFTWARE\Fortinet\FortiClient\Sslvpn\Tunnels";

    /// <remarks>Keys this user may not read count as no connection; the import then says there is none.</remarks>
    public static List<FortiClientConnection> Connections()
    {
        try
        {
            using var tunnels = Registry.LocalMachine.OpenSubKey(Tunnels);
            if (tunnels is null)
            {
                return [];
            }
            var connections = new List<FortiClientConnection>();
            foreach (var name in tunnels.GetSubKeyNames())
            {
                using var tunnel = tunnels.OpenSubKey(name);
                connections.Add(new FortiClientConnection(name, tunnel?.GetValue("Server") as string ?? ""));
            }
            return connections;
        }
        catch (Exception error) when (error is SecurityException or UnauthorizedAccessException or IOException)
        {
            return [];
        }
    }
}
