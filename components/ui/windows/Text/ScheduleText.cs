using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The schedule's sentences (components/ui/macos/Sentences/ScheduleText.swift).</summary>
public static class ScheduleText
{
    /// <summary>One line for the menu ("Schedule: …") and the Connection pane.</summary>
    public static string Summary(this ScheduleOutlook outlook, TimeStyle time)
    {
        ArgumentNullException.ThrowIfNull(time);
        return outlook switch
        {
            ScheduleOutlookConnects connects => L("Connects %@", time.DayTime(connects.At)),
            ScheduleOutlookNoDays => L("No days selected"),
            ScheduleOutlookWithinHours within => L("After %@, reconnects wait while you're away", time.ClockTime(within.End)),
            ScheduleOutlookOutsideHoursPresent => L("Reconnects wait while you're away"),
            ScheduleOutlookHoldingWhileAway holding => L("Reconnects stop if you're still away at %@", time.ClockTime(holding.Until)),
            ScheduleOutlookSuspended => L("Won't reconnect after this connection ends"),
            _ => throw new InvalidOperationException($"unknown outlook {outlook}"),
        };
    }

    public static string Message(this ScheduleHoursError error) => error switch
    {
        ScheduleHoursErrorEndNotAfterStart => L("The end time must be later than the start time."),
        ScheduleHoursErrorNotSaved notSaved => notSaved.Error.Message(),
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
        Weekday.Sun => L("Sun"),
        _ => throw new InvalidOperationException($"unknown weekday {day}"),
    };
}
