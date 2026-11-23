using System.Reflection;
using System.Text.Json.Serialization;
using Xunit;

namespace FortiVPNManager.Contract.Tests;

/// <summary>
/// The coding is generated when the app is built (MessageCoding): a case missing from its list, or a
/// type of cases or enum without its converter, fails only when such a value is sent, so every one
/// is checked here.
/// </summary>
public class MessageCodingTests
{
    private static readonly Type[] s_contractTypes = typeof(MessageCoding).Assembly.GetTypes();

    /// <summary>The abstract records whose nested records are their cases; static classes are abstract too, but sealed.</summary>
    private static readonly Type[] s_casesTypes = s_contractTypes.Where(type => type.IsAbstract && !type.IsSealed).ToArray();

    [Fact]
    public void EveryCaseIsListed()
    {
        Assert.Contains(typeof(AppInternal.ToRuntime), s_casesTypes);
        IEnumerable<string?> unlisted = s_casesTypes
            .SelectMany(type => type.GetNestedTypes().Where(nested => nested.IsSubclassOf(type)))
            .Where(@case => MessageCoding.Default.GetTypeInfo(@case) is null)
            .Select(@case => @case.FullName);
        Assert.Empty(unlisted);
    }

    [Fact]
    public void EveryTypeOfCasesOrEnumHasItsConverter()
    {
        IEnumerable<string?> unconverted = s_casesTypes
            .Concat(s_contractTypes.Where(type => type.IsEnum))
            .Where(type => type.GetCustomAttribute<JsonConverterAttribute>() is null)
            .Select(type => type.FullName);
        Assert.Empty(unconverted);
    }
}
