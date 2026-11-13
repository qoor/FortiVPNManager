using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The schedule's sentences (components/ui/macos/Sentences/ScheduleText.swift).</summary>
public static class ScheduleText
{
    /// <summary>One line for the menu ("Schedule: …") and the Connection pane.</summary>
    public static string Summary(this ScheduleOutlook outlook, TimeStyle time) => outlook switch
    {
        ScheduleOutlook.Connects connects => L("Connects %@", time.DayTime(connects.At)),
        ScheduleOutlook.NoDays => L("No days selected"),
        ScheduleOutlook.WithinHours within => L("After %@, reconnects wait while you're away", time.ClockTime(within.End)),
        ScheduleOutlook.OutsideHoursPresent => L("Reconnects wait while you're away"),
        ScheduleOutlook.HoldingWhileAway holding => L("Reconnects stop if you're still away at %@", time.ClockTime(holding.Until)),
        ScheduleOutlook.Suspended => L("Won't reconnect after this connection ends"),
        _ => throw new InvalidOperationException($"unknown outlook {outlook}"),
    };

    public static string Message(this ScheduleHoursError error) => error switch
    {
        ScheduleHoursError.EndNotAfterStart => L("The end time must be later than the start time."),
        ScheduleHoursError.NotSaved notSaved => notSaved.Error.Message(),
        _ => throw new InvalidOperationException($"unknown hours error {error}"),
    };

    /// <summary>Short weekday name for the day buttons.</summary>
    public static string ShortName(this Weekday day) => day switch
    {
        Weekday.Mon => L("Mon"),
        Weekday.Tue => L("Tue"),
        Weekday.Wed => L("Wed"),
        Weekday.Thu => L("Thu"),
        Weekday.Fri => L("Fri"),
        Weekday.Sat => L("Sat"),
        _ => L("Sun"),
    };
}
