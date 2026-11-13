using System.Globalization;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// How the UI writes times: relative to <paramref name="Now"/>, as clock times in the Windows region
/// format of <paramref name="Culture"/> and the local time zone. Times arrive as absolute instants;
/// every countdown and relative time is drawn here, as on macOS.
/// </summary>
public sealed record TimeStyle(DateTimeOffset Now, CultureInfo Culture)
{
    public static TimeStyle Current(DateTimeOffset now) => new(now, CultureInfo.CurrentCulture);

    /// <summary>A time value that came with a kind, such as a time limit: "50 seconds", "1 hour, 5 minutes".</summary>
    public string SpelledOut(ulong seconds)
    {
        var parts = new List<string>();
        var (hours, rest) = Math.DivRem(seconds, 3600UL);
        var (minutes, remaining) = Math.DivRem(rest, 60UL);
        if (hours > 0)
        {
            parts.Add(L("%lld hours", hours));
        }
        if (minutes > 0)
        {
            parts.Add(L("%lld minutes", minutes));
        }
        if (remaining > 0 || parts.Count == 0)
        {
            parts.Add(L("%lld seconds", remaining));
        }
        return parts.Aggregate((first, second) => L("%1$@, %2$@", first, second));
    }

    /// <summary>Whole minutes: "1 hr 23 min", "45 min", "2 hr", or "less than a minute".</summary>
    public string Duration(TimeSpan interval)
    {
        var minutes = (long)Math.Floor(interval.TotalMinutes);
        if (minutes < 1)
        {
            return L("less than a minute");
        }
        var (hours, rest) = Math.DivRem(minutes, 60L);
        if (hours == 0)
        {
            return L("%lld min", rest);
        }
        return rest == 0 ? L("%lld hr", hours) : L("%lld hr", hours) + " " + L("%lld min", rest);
    }

    /// <summary>"Reconnects in about 1 hr 32 min", or "Reconnects soon" within the last minute.</summary>
    public string PlannedReconnect(DateTimeOffset at)
    {
        var remaining = at - Now;
        return remaining < TimeSpan.FromMinutes(1)
            ? L("Reconnects soon")
            : L("Reconnects in about %@", Duration(remaining));
    }

    /// <summary>"1:12" (minutes:seconds, rounded up).</summary>
    public static string Countdown(TimeSpan interval)
    {
        var seconds = Math.Max(0L, (long)Math.Ceiling(interval.TotalSeconds));
        var (minutes, rest) = Math.DivRem(seconds, 60L);
        return string.Create(CultureInfo.InvariantCulture, $"{minutes}:{rest:00}");
    }

    /// <summary>The time today, otherwise with the weekday; for times within a week.</summary>
    public string DayTime(DateTimeOffset instant)
    {
        var local = instant.ToLocalTime();
        var time = local.ToString(Culture.DateTimeFormat.ShortTimePattern, Culture);
        return SameDay(local) ? time : local.ToString("ddd", Culture) + " " + time;
    }

    /// <summary>The time, with the month and day when it is not today and the year when not this year.</summary>
    public string ClockTime(DateTimeOffset instant)
    {
        var local = instant.ToLocalTime();
        var time = local.ToString(Culture.DateTimeFormat.ShortTimePattern, Culture);
        if (SameDay(local))
        {
            return time;
        }
        var date = local.Year == Now.ToLocalTime().Year
            ? local.ToString(Culture.DateTimeFormat.MonthDayPattern, Culture)
            : local.ToString(Culture.DateTimeFormat.ShortDatePattern, Culture);
        return date + " " + time;
    }

    private bool SameDay(DateTimeOffset local) => local.Date == Now.ToLocalTime().Date;
}
