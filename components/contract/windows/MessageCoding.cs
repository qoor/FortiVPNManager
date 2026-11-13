using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

/// <summary>
/// The JSON coding of every message to and from the runtime and over the Helper's pipe. It is
/// Swift's Codable coding, which the Rust side follows, so C# reads and writes the same shapes as
/// Swift: a type marked <see cref="CasesAttribute"/> is an object keyed by its case name, a value
/// without a label uses the key <c>_0</c>, a case without data is <c>{}</c>, times are whole
/// milliseconds since 1970, and absent optional values are left out. Decoding is strict: an
/// unknown key, a missing value or an unknown case is an error, since the app and the runtime are
/// one build.
/// </summary>
public static class MessageCoding
{
    public static JsonSerializerOptions Options { get; } = Create();

    private static JsonSerializerOptions Create()
    {
        var options = new JsonSerializerOptions
        {
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
            RespectNullableAnnotations = true,
            RespectRequiredConstructorParameters = true,
        };
        options.Converters.Add(new JsonStringEnumConverter(JsonNamingPolicy.CamelCase, allowIntegerValues: false));
        options.Converters.Add(new MillisecondsConverter());
        options.Converters.Add(new CasesConverterFactory());
        options.MakeReadOnly(populateMissingResolver: true);
        return options;
    }
}

/// <summary>
/// Marks an abstract record whose nested sealed records are its cases, as a Swift enum with
/// associated values: each case's name is its type's name in camel case.
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class CasesAttribute : Attribute;

internal sealed class MillisecondsConverter : JsonConverter<DateTimeOffset>
{
    public override DateTimeOffset Read(ref Utf8JsonReader reader, Type type, JsonSerializerOptions options) =>
        DateTimeOffset.FromUnixTimeMilliseconds(reader.GetInt64());

    public override void Write(Utf8JsonWriter writer, DateTimeOffset value, JsonSerializerOptions options) =>
        writer.WriteNumberValue(value.ToUnixTimeMilliseconds());
}

internal sealed class CasesConverterFactory : JsonConverterFactory
{
    public override bool CanConvert(Type type) =>
        type.IsAbstract && type.IsDefined(typeof(CasesAttribute), inherit: false);

    public override JsonConverter CreateConverter(Type type, JsonSerializerOptions options) =>
        (JsonConverter)Activator.CreateInstance(typeof(CasesConverter<>).MakeGenericType(type))!;
}

/// <summary>Writes a case as <c>{"caseName": {…its values…}}</c> and reads it back.</summary>
internal sealed class CasesConverter<T> : JsonConverter<T> where T : class
{
    private readonly Dictionary<string, Type> byName;
    private readonly Dictionary<Type, string> byType;

    public CasesConverter()
    {
        var cases = typeof(T)
            .GetNestedTypes(BindingFlags.Public)
            .Where(nested => nested.IsSubclassOf(typeof(T)));
        byName = cases.ToDictionary(nested => JsonNamingPolicy.CamelCase.ConvertName(nested.Name));
        byType = byName.ToDictionary(entry => entry.Value, entry => entry.Key);
    }

    public override T Read(ref Utf8JsonReader reader, Type type, JsonSerializerOptions options)
    {
        Expect(reader.TokenType == JsonTokenType.StartObject, "an object");
        reader.Read();
        Expect(reader.TokenType == JsonTokenType.PropertyName, "a case name");
        var name = reader.GetString()!;
        if (!byName.TryGetValue(name, out var caseType))
        {
            throw new JsonException($"{typeof(T).Name} has no case {name}");
        }
        reader.Read();
        // A case's values are an object, `{}` without any; null is no value of any case.
        Expect(reader.TokenType == JsonTokenType.StartObject, $"the values of {name}");
        var value = (T)JsonSerializer.Deserialize(ref reader, caseType, options)!;
        reader.Read();
        Expect(reader.TokenType == JsonTokenType.EndObject, "one case only");
        return value;
    }

    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
    {
        writer.WriteStartObject();
        writer.WritePropertyName(byType[value.GetType()]);
        JsonSerializer.Serialize(writer, value, value.GetType(), options);
        writer.WriteEndObject();
    }

    private static void Expect(bool condition, string what)
    {
        if (!condition)
        {
            throw new JsonException($"{typeof(T).Name}: expected {what}");
        }
    }
}
