using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization.Metadata;
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

    /// <remarks>The coding knows the messages' types, not lists of them, so the items are coded one by one.</remarks>
    public static List<T> RoundTrip<T>(string name)
    {
        JsonArray json = JsonNode.Parse(Read(name))!.AsArray();
        var coding = (JsonTypeInfo<T>)MessageCoding.Default.Options.GetTypeInfo(typeof(T));
        var values = json.Select(item => item.Deserialize(coding)!).ToList();
        var encoded = new JsonArray([.. values.Select(value => JsonSerializer.SerializeToNode(value, coding))]);
        Assert.True(JsonNode.DeepEquals(json, encoded), $"{name} encodes back as {encoded.ToJsonString()}");
        return values;
    }
}
