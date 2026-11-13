using Xunit;

namespace FortiVPNManager.OsIntegration.Tests;

/// <summary>Long log lines reach the event log whole.</summary>
public class SystemLogTests
{
    [Fact]
    public void ALongLineGoesWholeInParts()
    {
        // An emoji, two UTF-16 units, across the first part's end.
        var line = new string('a', SystemLog.MaxPart - 1) + "\U0001F600" + new string('b', SystemLog.MaxPart + 5);
        var parts = SystemLog.Parts(line);
        Assert.Equal(3, parts.Count);
        Assert.Equal(line, string.Concat(parts));
        Assert.All(parts, part => Assert.InRange(part.Length, 1, SystemLog.MaxPart));
        Assert.All(parts, part => Assert.False(char.IsHighSurrogate(part[^1])));
    }

    [Fact]
    public void AShortLineIsOnePart()
    {
        Assert.Equal([""], SystemLog.Parts(""));
        Assert.Equal(["short"], SystemLog.Parts("short"));
    }
}
