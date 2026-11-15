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
    private static readonly Type[] ContractTypes = typeof(MessageCoding).Assembly.GetTypes();

    /// <summary>The abstract records whose nested records are their cases; static classes are abstract too, but sealed.</summary>
    private static readonly Type[] CasesTypes = ContractTypes.Where(type => type.IsAbstract && !type.IsSealed).ToArray();

    [Fact]
    public void EveryCaseIsListed()
    {
        Assert.Contains(typeof(AppInternal.ToRuntime), CasesTypes);
        var unlisted = CasesTypes
            .SelectMany(type => type.GetNestedTypes().Where(nested => nested.IsSubclassOf(type)))
            .Where(@case => MessageCoding.Default.GetTypeInfo(@case) is null)
            .Select(@case => @case.FullName);
        Assert.Empty(unlisted);
    }

    [Fact]
    public void EveryTypeOfCasesOrEnumHasItsConverter()
    {
        var unconverted = CasesTypes
            .Concat(ContractTypes.Where(type => type.IsEnum))
            .Where(type => type.GetCustomAttribute<JsonConverterAttribute>() is null)
            .Select(type => type.FullName);
        Assert.Empty(unconverted);
    }
}
