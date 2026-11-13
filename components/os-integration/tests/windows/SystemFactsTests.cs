using System.Net;
using System.Net.NetworkInformation;
using Xunit;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration.Tests;

/// <summary>The network facts on the test PC, which has a wired network.</summary>
public class SystemFactsTests
{
    [Fact]
    public void TheRouteToAnAddressNamesTheInterfaceThatHasIt()
    {
        var loopback = SystemFacts.RouteInterface("127.0.0.1");
        Assert.NotNull(loopback);
        Assert.Contains(SystemFacts.InterfaceAddresses(), address => address.Name == loopback && address.Ipv4 == "127.0.0.1");
    }

    [Fact]
    public void ARouteIsLookedUpForIpv4AddressesOnly()
    {
        Assert.Null(SystemFacts.RouteInterface("vpn.example.com"));
        Assert.Null(SystemFacts.RouteInterface("::1"));
    }

    [Fact]
    public void TheDefaultRouterIsReachedThroughAnInterfaceThatIsUp()
    {
        var router = SystemFacts.PrimaryRouter();
        Assert.True(IPAddress.TryParse(router, out _), router);
        var name = SystemFacts.RouteInterface(router!);
        Assert.Contains(SystemFacts.PathInterfaces(), path => path.Name == name);
        Assert.True(SystemFacts.PathSatisfied());
    }

    [Fact]
    public void InterfaceKindsFollowTheTypes()
    {
        Assert.Equal(InterfaceKind.WiredEthernet, SystemFacts.Kind(NetworkInterfaceType.Ethernet));
        Assert.Equal(InterfaceKind.Wifi, SystemFacts.Kind(NetworkInterfaceType.Wireless80211));
        Assert.Equal(InterfaceKind.Loopback, SystemFacts.Kind(NetworkInterfaceType.Loopback));
        Assert.Equal(InterfaceKind.Other, SystemFacts.Kind(NetworkInterfaceType.Tunnel));
        Assert.Equal(InterfaceKind.Other, SystemFacts.Kind(NetworkInterfaceType.Ppp));
    }

    [Fact]
    public void TheTimeZoneIsAnIanaName()
    {
        Assert.Contains('/', SystemEvents.IanaTimeZone());
    }
}
