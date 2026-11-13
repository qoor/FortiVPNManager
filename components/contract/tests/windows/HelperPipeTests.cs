using System.Text;
using Xunit;

namespace FortiVPNManager.Contract.Tests;

/// <summary>
/// The C# types of the Helper's pipe against ../fixtures/helper-pipe, and the framing every message
/// uses on the pipes.
/// </summary>
public class HelperPipeTests
{
    [Fact]
    public void RequestsFromTheApp()
    {
        var requests = Fixtures.RoundTrip<HelperPipe.Request>("helper-pipe/requests.json");
        Assert.Equal(new HelperPipe.Request.Build(1, "20261112.1"), requests[0]);
        Assert.Equal(8, requests.Count);
    }

    [Fact]
    public void MessagesToTheApp()
    {
        var messages = Fixtures.RoundTrip<HelperPipe.Message>("helper-pipe/messages.json");
        Assert.Equal(new HelperPipe.Message.Build(1, "20261112.1"), messages[0]);
        Assert.Equal(new HelperPipe.Message.Reply(2), messages[1]);
    }

    [Fact]
    public void DebugOutputLeavesSecretsOut()
    {
        var connect = new HelperPipe.Request.Connect(2, """{"password":"vpn-secret"}""").ToString();
        var otp = new HelperPipe.Request.SubmitOtp(3, "attempt", "123456").ToString();
        Assert.DoesNotContain("vpn-secret", connect);
        Assert.DoesNotContain("123456", otp);
    }

    [Fact]
    public void RequestsAreFramedWithABigEndianLength()
    {
        using var stream = new MemoryStream();
        // Passed as the case, it is still written inside its type's wrapper.
        MessageFraming.Write(stream, new HelperPipe.Request.Status(5));
        var bytes = stream.ToArray();
        const string payload = """{"status":{"id":5}}""";
        Assert.Equal(new byte[] { 0, 0, 0, (byte)payload.Length }, bytes[..4]);
        Assert.Equal(payload, Encoding.UTF8.GetString(bytes, 4, bytes.Length - 4));
    }

    [Fact]
    public void MessagesAreReadOneFrameAtATime()
    {
        using var stream = new MemoryStream();
        foreach (var json in new[] { """{"replyFlag":{"id":6,"value":false}}""", """{"output":{"json":"[]"}}""" })
        {
            var payload = Encoding.UTF8.GetBytes(json);
            stream.Write([0, 0, (byte)(payload.Length >> 8), (byte)payload.Length]);
            stream.Write(payload);
        }
        stream.Position = 0;
        Assert.Equal(new HelperPipe.Message.ReplyFlag(6, false), MessageFraming.ReadFromHelper(stream));
        Assert.Equal(new HelperPipe.Message.Output("[]"), MessageFraming.ReadFromHelper(stream));
        Assert.Throws<EndOfStreamException>(() => MessageFraming.ReadFromHelper(stream));
    }
}
