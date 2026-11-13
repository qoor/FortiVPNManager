using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// Point-in-time facts about the network, as Windows reports them. Interfaces are named as Windows
/// shows them, such as "Ethernet", in every fact and event, so the runtime can match them up.
/// </summary>
internal static class SystemFacts
{
    [StructLayout(LayoutKind.Sequential)]
    private struct ForwardRow
    {
        public uint Destination;
        public uint Mask;
        public uint Policy;
        public uint NextHop;
        public uint InterfaceIndex;
        public uint Type;
        public uint Protocol;
        public uint Age;
        public uint NextHopAs;
        public uint Metric1;
        public uint Metric2;
        public uint Metric3;
        public uint Metric4;
        public uint Metric5;
    }

    [DllImport("iphlpapi.dll")]
    private static extern uint GetBestInterface(uint destination, out uint interfaceIndex);

    [DllImport("iphlpapi.dll")]
    private static extern uint GetBestRoute(uint destination, uint source, out ForwardRow route);

    /// <summary>The interface the routing table uses for <paramref name="destination"/>, an IPv4 address.</summary>
    public static string? RouteInterface(string destination)
    {
        if (!IPAddress.TryParse(destination, out var address) || address.AddressFamily != AddressFamily.InterNetwork)
        {
            return null;
        }
        if (GetBestInterface(NetworkOrder(address), out var index) != 0)
        {
            return null;
        }
        return NetworkInterface.GetAllNetworkInterfaces().FirstOrDefault(nic => Ipv4Index(nic) == index)?.Name;
    }

    /// <summary>Every IPv4 address of every interface; an interface without one appears once without an address.</summary>
    public static IReadOnlyList<InterfaceAddress> InterfaceAddresses() =>
        NetworkInterface.GetAllNetworkInterfaces()
            .SelectMany(nic =>
            {
                var up = nic.OperationalStatus == OperationalStatus.Up;
                var addresses = nic.GetIPProperties().UnicastAddresses
                    .Where(unicast => unicast.Address.AddressFamily == AddressFamily.InterNetwork)
                    .Select(unicast => new InterfaceAddress(nic.Name, up, unicast.Address.ToString()))
                    .ToList();
                return addresses.Count > 0 ? addresses : [new InterfaceAddress(nic.Name, up)];
            })
            .ToList();

    /// <summary>The next hop of the default route the routing table picks.</summary>
    public static string? PrimaryRouter() =>
        GetBestRoute(0, 0, out var route) == 0 && route.NextHop != 0
            ? new IPAddress(route.NextHop).ToString()
            : null;

    /// <summary>Whether a network other than loopback is up.</summary>
    public static bool PathSatisfied() => NetworkInterface.GetIsNetworkAvailable();

    /// <summary>The interfaces that are up.</summary>
    public static IReadOnlyList<PathInterface> PathInterfaces() =>
        NetworkInterface.GetAllNetworkInterfaces()
            .Where(nic => nic.OperationalStatus == OperationalStatus.Up)
            .Select(nic => new PathInterface(nic.Name, Kind(nic.NetworkInterfaceType)))
            .ToList();

    /// <summary>A tunnel or any other interface counts as other.</summary>
    public static InterfaceKind Kind(NetworkInterfaceType type) => type switch
    {
        NetworkInterfaceType.Ethernet or NetworkInterfaceType.Ethernet3Megabit or NetworkInterfaceType.FastEthernetFx
            or NetworkInterfaceType.FastEthernetT or NetworkInterfaceType.GigabitEthernet => InterfaceKind.WiredEthernet,
        NetworkInterfaceType.Wireless80211 => InterfaceKind.Wifi,
        NetworkInterfaceType.Wwanpp or NetworkInterfaceType.Wwanpp2 => InterfaceKind.Cellular,
        NetworkInterfaceType.Loopback => InterfaceKind.Loopback,
        _ => InterfaceKind.Other,
    };

    /// <summary>The address as the routing functions take it: its bytes in network order.</summary>
    private static uint NetworkOrder(IPAddress address) => BitConverter.ToUInt32(address.GetAddressBytes());

    private static uint? Ipv4Index(NetworkInterface nic)
    {
        try
        {
            return (uint?)nic.GetIPProperties().GetIPv4Properties()?.Index;
        }
        catch (NetworkInformationException)
        {
            // IPv4 is off on this interface.
            return null;
        }
    }
}
