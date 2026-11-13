using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace FortiVPNManager.Contract.Tests;

/// <summary>
/// The JSON in ../fixtures, which the Rust tests pin. Every fixture is a list that must decode into
/// the C# types and encode back into the same JSON: the coding is strict, so a missing or misnamed
/// key, case or value fails one way or the other.
/// </summary>
internal static class Fixtures
{
    public static string Read(string name, [CallerFilePath] string here = "") =>
        File.ReadAllText(Path.Combine(Path.GetDirectoryName(here)!, "..", "fixtures", name));

    public static List<T> RoundTrip<T>(string name)
    {
        var json = Read(name);
        var values = JsonSerializer.Deserialize<List<T>>(json, MessageCoding.Options)!;
        var encoded = JsonSerializer.SerializeToNode(values, MessageCoding.Options);
        Assert.True(
            JsonNode.DeepEquals(JsonNode.Parse(json), encoded),
            $"{name} encodes back as {encoded?.ToJsonString()}");
        return values;
    }
}
