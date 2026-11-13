using System.Globalization;
using System.Reflection;
using System.Text;
using System.Text.Json;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// The app's sentences in the Windows display language, Korean or English. The shared sentences come
/// from the macOS string catalog (components/ui/macos/Localizable.xcstrings), their one source; the
/// Windows catalog holds the sentences that name Windows where the macOS ones name macOS, and the
/// time units macOS takes from the system. Keys are the English sentences with Apple's format
/// specifiers (%@ for text, %lld for a number), as the catalogs store them.
/// </summary>
public static class Strings
{
    private static readonly Lazy<Catalog> Windows = new(() => Catalog.Load("Windows.xcstrings"));
    private static readonly Lazy<Catalog> Shared = new(() => Catalog.Load("Localizable.xcstrings"));

    /// <summary>"ko" when Windows shows Korean, otherwise "en", the catalogs' source language.</summary>
    public static string Language { get; set; } =
        CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "ko" ? "ko" : "en";

    /// <summary>The sentence <paramref name="key"/> with <paramref name="arguments"/> in its specifiers' places.</summary>
    public static string L(string key, params object[] arguments)
    {
        var unit = Windows.Value.Find(key, Language) ?? Shared.Value.Find(key, Language);
        var template = unit is null ? key : unit.Choose(arguments);
        return Format(template, arguments);
    }

    /// <summary>Whether a catalog has <paramref name="key"/>, for the test that every key used exists.</summary>
    internal static bool Has(string key) => Windows.Value.Has(key) || Shared.Value.Has(key);

    /// <summary>Replaces Apple's specifiers, in order or by position (%2$@), with the arguments.</summary>
    internal static string Format(string template, object[] arguments)
    {
        var text = new StringBuilder();
        var next = 0;
        for (var i = 0; i < template.Length; i++)
        {
            if (template[i] != '%' || i + 1 == template.Length)
            {
                text.Append(template[i]);
                continue;
            }
            if (template[i + 1] == '%')
            {
                text.Append('%');
                i++;
                continue;
            }
            var end = i + 1;
            var position = -1;
            while (end < template.Length && char.IsAsciiDigit(template[end]))
            {
                end++;
            }
            if (end < template.Length && template[end] == '$' && end > i + 1)
            {
                position = int.Parse(template.AsSpan(i + 1, end - i - 1), CultureInfo.InvariantCulture) - 1;
                end++;
            }
            else
            {
                end = i + 1;
            }
            // The conversion: @, or an integer's length modifiers and letter.
            while (end < template.Length && template[end] is 'l' or 'h' or 'q' or 'z')
            {
                end++;
            }
            if (end == template.Length || !(template[end] is '@' or 'd' or 'i' or 'u'))
            {
                text.Append(template[i]);
                continue;
            }
            var index = position >= 0 ? position : next++;
            text.Append(index < arguments.Length ? Display(arguments[index]) : string.Empty);
            i = end;
        }
        return text.ToString();
    }

    /// <summary>Numbers without digit grouping, as macOS writes them in these sentences.</summary>
    private static string Display(object value) => value switch
    {
        IFormattable number => number.ToString(null, CultureInfo.InvariantCulture),
        _ => value.ToString() ?? string.Empty,
    };

    /// <summary>One catalog: for each key, its sentence in each language that has one.</summary>
    private sealed class Catalog(Dictionary<string, Dictionary<string, Unit>> entries)
    {
        public static Catalog Load(string name)
        {
            using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream(name)
                ?? throw new InvalidOperationException($"the string catalog {name} is missing from the build");
            using var document = JsonDocument.Parse(stream);
            var entries = new Dictionary<string, Dictionary<string, Unit>>(StringComparer.Ordinal);
            foreach (var entry in document.RootElement.GetProperty("strings").EnumerateObject())
            {
                var languages = new Dictionary<string, Unit>(StringComparer.Ordinal);
                if (entry.Value.TryGetProperty("localizations", out var localizations))
                {
                    foreach (var language in localizations.EnumerateObject())
                    {
                        if (Unit.Read(language.Value) is { } unit)
                        {
                            languages[language.Name] = unit;
                        }
                    }
                }
                entries[entry.Name] = languages;
            }
            return new Catalog(entries);
        }

        public bool Has(string key) => entries.ContainsKey(key);

        /// <summary>The sentence in <paramref name="language"/>, or null to use the key itself.</summary>
        public Unit? Find(string key, string language) =>
            entries.TryGetValue(key, out var languages) && languages.TryGetValue(language, out var unit) ? unit : null;
    }

    /// <summary>A sentence, or its plural forms keyed one, other and so on.</summary>
    private sealed record Unit(string? Value, Dictionary<string, string>? Plural)
    {
        public static Unit? Read(JsonElement localization)
        {
            if (localization.TryGetProperty("stringUnit", out var stringUnit))
            {
                return new Unit(stringUnit.GetProperty("value").GetString(), null);
            }
            if (localization.TryGetProperty("variations", out var variations)
                && variations.TryGetProperty("plural", out var plural))
            {
                var forms = new Dictionary<string, string>(StringComparer.Ordinal);
                foreach (var form in plural.EnumerateObject())
                {
                    forms[form.Name] = form.Value.GetProperty("stringUnit").GetProperty("value").GetString() ?? string.Empty;
                }
                return new Unit(null, forms);
            }
            return null;
        }

        /// <summary>The plural form for the first number among the arguments: one for 1, else other.</summary>
        public string Choose(object[] arguments)
        {
            if (Plural is null)
            {
                return Value ?? string.Empty;
            }
            var count = arguments.FirstOrDefault(argument => argument is long or int or ulong or uint);
            var form = count is not null && Convert.ToDecimal(count, CultureInfo.InvariantCulture) == 1 ? "one" : "other";
            return Plural.TryGetValue(form, out var chosen) ? chosen : Plural.GetValueOrDefault("other", string.Empty);
        }
    }
}
