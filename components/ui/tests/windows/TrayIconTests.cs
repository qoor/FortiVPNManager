using Xunit;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui.Tests;

public class TrayIconTests
{
    [Fact]
    public void EveryStateHasItsIconFile()
    {
        IEnumerable<string> missing = Enum.GetValues<ConnectionIcon>()
            .Select(TrayIcon.IconFile)
            .Where(file => !File.Exists(Path.Combine(StringsTests.s_repository, "packaging", "windows", file)));
        Assert.Empty(missing);
    }
}
