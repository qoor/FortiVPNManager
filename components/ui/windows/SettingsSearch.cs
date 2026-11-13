using System.Globalization;
using System.Text;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui;

/// <summary>The state a setting's row is shown in, when only in some.</summary>
public enum SearchCondition
{
    /// <summary>The VPN is connected (the General pane's connection details).</summary>
    Connected,

    /// <summary>The OTP is read with the Gmail API (the Account pane's Google rows).</summary>
    GmailApi,

    /// <summary>The VPN client was looked for (the Advanced pane's client rows).</summary>
    VpnClientFound,

    /// <summary>The Advanced pane offers Register Helper.</summary>
    RegisterHelper,

    /// <summary>The Exception Rules pane offers Add Current Network.</summary>
    AddCurrentNetwork,
}

/// <summary>
/// A setting the search finds: its pane, its English label (also its catalog key), the label in the
/// app's language, the row it scrolls to (its own or one it shares; none for a command at the top of
/// the pane), and the state its row is shown in. The pane opens the expander a row is in.
/// </summary>
public sealed record SearchItem(SettingsPane Pane, string Key, string Title, string? Row, SearchCondition? Condition)
{
    internal SettingsSearchMatch.Name Name { get; } =
        new($"{L(Pane.TitleKey())} {Pane.TitleKey()} {Title} {Key}");
}

/// <summary>A result: a pane whose name matched (no item), or a setting in it.</summary>
public sealed record SearchResult(SettingsPane Pane, SearchItem? Item);

/// <summary>
/// The settings search (components/ui/macos/SettingsNavigation.swift, docs/SETTINGS-UI.md §5): pane
/// and setting names in the app's language and in English, every term matching. A pane whose own
/// name matches is the result itself, so it lists none of its settings; settings the panes don't show
/// now are left out.
/// </summary>
public static class SettingsSearch
{
    /// <summary>Rows that hold more than one searchable setting.</summary>
    public const string ConnectionButtons = "Connect";
    public const string ImportRow = "FortiClient settings";
    public const string OAuthClientButtons = "Import JSON…";

    private static readonly Dictionary<SettingsPane, List<SearchItem>> Index =
        Enum.GetValues<SettingsPane>().ToDictionary(pane => pane, Items);

    public static IReadOnlyList<SearchItem> ItemsIn(SettingsPane pane) => Index[pane];

    /// <summary>The results for <paramref name="text"/>, in the panes' order; none for an empty search.</summary>
    public static List<SearchResult> Results(string text, Func<SearchCondition, bool> isShown)
    {
        var terms = text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).Select(term => new SettingsSearchMatch.Term(term)).ToList();
        var results = new List<SearchResult>();
        if (terms.Count == 0)
        {
            return results;
        }
        foreach (var pane in Enum.GetValues<SettingsPane>())
        {
            if (terms.All(term => SettingsSearchMatch.Matches(term, PaneName(pane))))
            {
                results.Add(new SearchResult(pane, null));
                continue;
            }
            results.AddRange(Index[pane]
                .Where(item => (item.Condition is not { } condition || isShown(condition)) && terms.All(term => SettingsSearchMatch.Matches(term, item.Name)))
                .Select(item => new SearchResult(pane, item)));
        }
        return results;
    }

    private static SettingsSearchMatch.Name PaneName(SettingsPane pane) => new($"{L(pane.TitleKey())} {pane.TitleKey()}");

    private static List<SearchItem> Items(SettingsPane pane) => pane switch
    {
        SettingsPane.General =>
        [
            Item(pane, "Current status"),
            Item(pane, "Connected at", when: SearchCondition.Connected),
            Item(pane, "Tunnel IP", when: SearchCondition.Connected),
            Item(pane, "Next proactive reconnect", when: SearchCondition.Connected),
            Item(pane, "Connect", row: ConnectionButtons),
            Item(pane, "Disconnect", row: ConnectionButtons),
            Item(pane, "Reconnect Now", row: ConnectionButtons),
            Item(pane, "Open at login"),
            Item(pane, "Connect automatically at launch"),
            Item(pane, "FortiClient settings"),
            Item(pane, "Import Settings", row: ImportRow),
        ],
        SettingsPane.Account =>
        [
            Item(pane, "Server address"),
            Item(pane, "Port"),
            Item(pane, "Username"),
            Item(pane, "Password"),
            Item(pane, "Realm"),
            Item(pane, "Gmail address"),
            Item(pane, "Connection method"),
            Item(pane, "Google account", when: SearchCondition.GmailApi),
            Item(pane, "IMAP fallback (optional)", when: SearchCondition.GmailApi),
            Item(pane, "App password"),
            Item(pane, "OAuth client", when: SearchCondition.GmailApi),
            Item(pane, "Import JSON…", row: OAuthClientButtons, when: SearchCondition.GmailApi),
            Item(pane, "Use Built-in Client", row: OAuthClientButtons, when: SearchCondition.GmailApi),
            Item(pane, "Test Gmail Connection"),
        ],
        SettingsPane.Connection =>
        [
            Item(pane, "Connect on a schedule"),
            Item(pane, "Days"),
            Item(pane, "Start time"),
            Item(pane, "End time"),
            Item(pane, "Stop reconnecting after being away"),
            Item(pane, "Maximum session length"),
            Item(pane, "Proactive reconnect"),
            Item(pane, "Margin"),
            Item(pane, "Reconnect after sleep"),
            Item(pane, "Minimum sleep time"),
        ],
        SettingsPane.ExceptionRules =>
        [
            Item(pane, "Current network"),
            Item(pane, "Status"),
            Item(pane, "Networks where the rules apply"),
            Item(pane, "Add Current Network", when: SearchCondition.AddCurrentNetwork),
            Item(pane, "Exception targets"),
        ],
        SettingsPane.Advanced =>
        [
            Item(pane, "Helper"),
            Item(pane, "Register Helper", when: SearchCondition.RegisterHelper),
            Item(pane, "Type", when: SearchCondition.VpnClientFound),
            // The client's name is not translated.
            new SearchItem(pane, "openconnect", "openconnect", "Type", SearchCondition.VpnClientFound),
            Item(pane, "Version", when: SearchCondition.VpnClientFound),
            Item(pane, "Path", when: SearchCondition.VpnClientFound),
            Item(pane, "Settings File"),
            Item(pane, "Show in File Explorer", row: "Settings File"),
        ],
        SettingsPane.Log =>
        [
            Item(pane, "Auto-scroll"),
            Item(pane, "Find", command: true),
            Item(pane, "Clear", command: true),
            Item(pane, "Copy All", command: true),
            Item(pane, "Show in File Explorer", command: true),
        ],
        _ => throw new InvalidOperationException($"unknown pane {pane}"),
    };

    /// <summary>A setting labelled <paramref name="key"/>, in its own row unless it shares one or is a command at the top.</summary>
    private static SearchItem Item(SettingsPane pane, string key, string? row = null, SearchCondition? when = null, bool command = false) =>
        new(pane, key, L(key), command ? null : row ?? key, when);
}

/// <summary>
/// Settings search matching, measured against macOS 27 System Settings in Korean (both typed with the
/// 2-Set Korean input method and set directly), as components/ui/macos/SettingsNavigation.swift does:
/// characters before the last match as whole syllables anywhere in a name; the last, which may still
/// be composing, matches at the jamo level (a lone consonant starts a syllable, a syllable begins
/// one); a one-syllable term's final consonant may be the next syllable's first one at the start of a
/// word only. Earlier jamo, initial-consonant abbreviations and Hangul spellings of English words don't
/// match; Latin text matches case-insensitively.
/// </summary>
public static class SettingsSearchMatch
{
    /// <summary>A name prepared once for matching: normalized, with each character's jamo.</summary>
    public sealed class Name
    {
        internal Name(string text)
        {
            Characters = Elements(text);
            Jamo = Characters.Select(SettingsSearchMatch.Jamo).ToArray();
        }

        internal string[] Characters { get; }

        internal string[] Jamo { get; }
    }

    /// <summary>A term prepared once per search: normalized, with the jamo of its last character.</summary>
    public sealed class Term
    {
        public Term(string term)
        {
            Characters = Elements(term);
            LastJamo = Characters.Length > 0 ? SettingsSearchMatch.Jamo(Characters[^1]) : "";
        }

        internal string[] Characters { get; }

        internal string LastJamo { get; }
    }

    public static bool Matches(string term, string text) => Matches(new Term(term), new Name(text));

    public static bool Matches(Term prepared, Name name)
    {
        var term = prepared.Characters;
        var text = name.Characters;
        if (term.Length == 0)
        {
            return true;
        }
        var last = term[^1];
        var head = term[..^1];
        var partial = IsHangul(last);
        for (var start = 0; start + term.Length <= text.Length; start++)
        {
            if (!head.AsSpan().SequenceEqual(text.AsSpan(start, head.Length)))
            {
                continue;
            }
            var end = start + head.Length;
            if (text[end] == last || (partial && name.Jamo[end].StartsWith(prepared.LastJamo, StringComparison.Ordinal)))
            {
                return true;
            }
        }
        // 뱉 while typing 배터리: the final consonant is the next syllable's first one.
        if (term.Length != 1 || SyllableIndex(last) is not { } index || index % 28 == 0 || text.Length < 2)
        {
            return false;
        }
        var baseJamo = prepared.LastJamo[..^1];
        var next = prepared.LastJamo[^1..];
        for (var start = 0; start < text.Length - 1; start++)
        {
            if (start > 0 && char.IsLetterOrDigit(text[start - 1][0]))
            {
                continue;
            }
            if (name.Jamo[start] == baseJamo && name.Jamo[start + 1].StartsWith(next, StringComparison.Ordinal))
            {
                return true;
            }
        }
        return false;
    }

    private static readonly string[] Initials = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ".Select(c => c.ToString()).ToArray();
    private static readonly string[] Medials = ["ㅏ", "ㅐ", "ㅑ", "ㅒ", "ㅓ", "ㅔ", "ㅕ", "ㅖ", "ㅗ", "ㅗㅏ", "ㅗㅐ", "ㅗㅣ", "ㅛ", "ㅜ", "ㅜㅓ", "ㅜㅔ", "ㅜㅣ", "ㅠ", "ㅡ", "ㅡㅣ", "ㅣ"];
    private static readonly string[] Finals = ["", "ㄱ", "ㄲ", "ㄱㅅ", "ㄴ", "ㄴㅈ", "ㄴㅎ", "ㄷ", "ㄹ", "ㄹㄱ", "ㄹㅁ", "ㄹㅂ", "ㄹㅅ", "ㄹㅌ", "ㄹㅍ", "ㄹㅎ", "ㅁ", "ㅂ", "ㅂㅅ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ"];
    private static readonly Dictionary<string, string> Compounds = new()
    {
        ["ㄳ"] = "ㄱㅅ", ["ㄵ"] = "ㄴㅈ", ["ㄶ"] = "ㄴㅎ", ["ㄺ"] = "ㄹㄱ", ["ㄻ"] = "ㄹㅁ", ["ㄼ"] = "ㄹㅂ", ["ㄽ"] = "ㄹㅅ", ["ㄾ"] = "ㄹㅌ", ["ㄿ"] = "ㄹㅍ", ["ㅀ"] = "ㄹㅎ", ["ㅄ"] = "ㅂㅅ",
        ["ㅘ"] = "ㅗㅏ", ["ㅙ"] = "ㅗㅐ", ["ㅚ"] = "ㅗㅣ", ["ㅝ"] = "ㅜㅓ", ["ㅞ"] = "ㅜㅔ", ["ㅟ"] = "ㅜㅣ", ["ㅢ"] = "ㅡㅣ",
    };

    /// <summary>The text's characters as the reader sees them (as Swift's Character), composed and lowercased.</summary>
    private static string[] Elements(string text)
    {
        var normalized = text.Normalize(NormalizationForm.FormC).ToLowerInvariant();
        var elements = new List<string>();
        var enumerator = StringInfo.GetTextElementEnumerator(normalized);
        while (enumerator.MoveNext())
        {
            elements.Add(enumerator.GetTextElement());
        }
        return elements.ToArray();
    }

    private static int? SyllableIndex(string character) =>
        character.Length == 1 && character[0] is >= '가' and <= '힣' ? character[0] - 0xAC00 : null;

    private static bool IsHangul(string character) =>
        SyllableIndex(character) is not null || character.All(c => c is >= 'ㄱ' and <= 'ㆎ');

    /// <summary>Compatibility jamo with compound vowels and finals split, so a composing syllable is a prefix of its result.</summary>
    private static string Jamo(string character) =>
        SyllableIndex(character) is { } index
            ? Initials[index / 588] + Medials[index % 588 / 28] + Finals[index % 28]
            : Compounds.GetValueOrDefault(character, character);
}
