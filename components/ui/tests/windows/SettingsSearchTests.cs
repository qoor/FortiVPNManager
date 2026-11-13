using Xunit;

namespace FortiVPNManager.Ui.Tests;

/// <summary>
/// The settings search (components/ui/tests/macos/SettingsSearchTests.swift): the Korean matching
/// measured against macOS 27 System Settings, and what the search lists.
/// </summary>
public class SettingsSearchTests
{
    /// <summary>Each term against a name, as macOS 27 System Settings in Korean matched it.</summary>
    [Theory]
    [InlineData("배터리", "배터리 성능 상태", true)]
    [InlineData("뱉", "배터리 성능 상태", true)]
    [InlineData("배털", "배터리 성능 상태", false)]
    [InlineData("배텨", "배터리", false)]
    [InlineData("배터ㄹ", "배터리", true)]
    [InlineData("배ㅌ", "배터리", true)]
    [InlineData("디스프", "디스플레이", true)]
    [InlineData("디스플렝", "디스플레이", false)]
    [InlineData("디슾", "디스플레이", false)]
    [InlineData("디스플레ㅇ", "디스플레이", true)]
    [InlineData("ㄷ", "디스플레이", true)]
    [InlineData("ㄷ", "사운드", true)]
    [InlineData("ㅂㅌㄹ", "배터리", false)]
    [InlineData("ㄷ스플레이", "디스플레이", false)]
    [InlineData("디ㅅ플레이", "디스플레이", false)]
    [InlineData("털", "배터리", false)]
    [InlineData("렝", "키보드 레이아웃", true)]
    [InlineData("슾", "스피커", true)]
    [InlineData("터ㄹ", "배터리", true)]
    [InlineData("스플레", "디스플레이", true)]
    [InlineData("배ㅌㅓ", "배터리", false)]
    [InlineData("batt", "Battery", true)]
    [InlineData("BATTERY", "배터리 Battery", true)]
    [InlineData("디습", "디스플레이", false)]
    [InlineData("블루투스", "Bluetooth", false)]
    [InlineData("렝", "디스플레이", false)]
    public void MatchesLikeSystemSettings(string term, string name, bool matches) =>
        Assert.Equal(matches, SettingsSearchMatch.Matches(term, name));

    private static List<SearchResult> Search(string text, Func<SearchCondition, bool>? isShown = null) =>
        SettingsSearch.Results(text, isShown ?? (_ => true));

    private static List<string> Keys(List<SearchResult> results, SettingsPane pane) =>
        results.Where(result => result.Pane == pane && result.Item is not null).Select(result => result.Item!.Key).ToList();

    [Fact]
    public void ListsTheMatchingSettingsWithTheirPane()
    {
        Assert.Contains("Port", Keys(Search("port"), SettingsPane.Account));
        Assert.Empty(Search("zzzz"));
        Assert.Empty(Search("  "));
    }

    [Fact]
    public void APaneMatchedByItsNameIsTheResultItself()
    {
        var results = Search("exception rules");
        Assert.Contains(new SearchResult(SettingsPane.ExceptionRules, null), results);
        Assert.Empty(Keys(results, SettingsPane.ExceptionRules));
    }

    [Fact]
    public void ListsOnlyTheSettingsThePanesShowNow()
    {
        Assert.Empty(Keys(Search("oauth", condition => condition != SearchCondition.GmailApi), SettingsPane.Account));
        Assert.Contains("OAuth client", Keys(Search("oauth"), SettingsPane.Account));
    }

    [Fact]
    public void EveryTermMustMatch() =>
        Assert.Equal(["Test Gmail Connection"], Keys(Search("gmail test"), SettingsPane.Account));

    [Fact]
    public void SettingsSharingARowGoToIt()
    {
        var reconnect = Assert.Single(Search("reconnect now"), result => result.Item?.Key == "Reconnect Now");
        Assert.Equal(SettingsSearch.ConnectionButtons, reconnect.Item!.Row);
        // A command at the top of a pane has no row; choosing it only opens the pane.
        var find = Assert.Single(Search("find"), result => result.Pane == SettingsPane.Log);
        Assert.Null(find.Item!.Row);
    }

    [Fact]
    public void EverySettingIsInACatalog()
    {
        var missing = Enum.GetValues<SettingsPane>()
            .SelectMany(SettingsSearch.ItemsIn)
            .Where(item => item.Key != "openconnect" && !Text.Strings.Has(item.Key))
            .Select(item => item.Key)
            .ToList();
        Assert.Empty(missing);
    }
}
