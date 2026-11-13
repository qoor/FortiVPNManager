using FortiVPNManager.Contract;
using FortiVPNManager.Ui.Settings;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Dispatching;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui;

/// <summary>
/// What the menu and the settings window draw: the runtime's latest domain sections and log lines,
/// as they came (components/ui/macos/AppUI.swift). The user's actions go back as commands. The
/// runtime calls in on its reader thread; everything here then runs on the UI thread, in order.
/// </summary>
public sealed class AppUi
{
    private readonly DispatcherQueue dispatcher;
    private readonly Action<string, string, string> notify;
    private readonly DispatcherQueueTimer ticker;
    private SettingsWindow? settings;

    /// <param name="send">Sends a command to the runtime.</param>
    /// <param name="notify">Posts a notification as (identifier, title, body), replacing one of the same identifier.</param>
    public AppUi(DispatcherQueue dispatcher, Action<Command> send, Action<string, string, string> notify)
    {
        this.dispatcher = dispatcher;
        this.notify = notify;
        Send = send;
        ticker = dispatcher.CreateTimer();
        ticker.Tick += (_, _) => Tick();
    }

    public Action<Command> Send { get; }

    public ConnectionSection? Connection { get; private set; }

    public ScheduleSection? Schedule { get; private set; }

    public ReadinessSection? Readiness { get; private set; }

    public GmailSection? Gmail { get; private set; }

    public BypassRulesSection? BypassRules { get; private set; }

    public ConfigSection? Config { get; private set; }

    /// <summary>The lines the Log pane shows; the log file keeps them all.</summary>
    public LogLines Log { get; } = new();

    public AccountDraft AccountDraft { get; } = new();

    /// <summary>The Account pane's disclosures the person opened, kept while the app runs (macOS SettingsNavigation).</summary>
    public HashSet<string> OpenDisclosures { get; } = [];

    /// <summary>The Log pane's Auto-scroll, kept while the app runs.</summary>
    public bool LogAutoScroll { get; set; } = true;

    /// <summary>"Now" for relative times: set by every connection section, every 30 s, and every second of a countdown.</summary>
    public DateTimeOffset Now { get; private set; } = DateTimeOffset.Now;

    public TimeStyle Time => TimeStyle.Current(Now);

    /// <summary>A section changed or the clock ticked.</summary>
    public event Action? Changed;

    /// <summary>Lines were added to the log.</summary>
    public event Action<IReadOnlyList<LogLine>>? LogAppended;

    /// <summary>The settings window, made the first time it is needed.</summary>
    public SettingsWindow Settings => settings ??= new SettingsWindow(this);

    /// <summary>Hides the settings window, when it was ever made; quitting doesn't make one only to hide it.</summary>
    public void HideSettings() => settings?.Hide();

    /// <summary>Sends what Settings still has in its fields; quitting asks the runtime only after it.</summary>
    public void FinishSettingsTyping() => settings?.FinishTyping();

    // From the runtime, on its reader thread.

    public void Apply(Section section) => dispatcher.TryEnqueue(() =>
    {
        switch (section)
        {
            case Section.Connection connection:
                Connection = connection.Value;
                RestartClock();
                break;
            case Section.Schedule schedule:
                Schedule = schedule.Value;
                break;
            case Section.Readiness readiness:
                Readiness = readiness.Value;
                break;
            case Section.Gmail gmail:
                Gmail = gmail.Value;
                break;
            case Section.BypassRules rules:
                BypassRules = rules.Value;
                break;
            case Section.Config config:
                Config = config.Value;
                AccountDraft.Replace(config.Value.Account);
                break;
        }
        Changed?.Invoke();
    });

    public void AppendLog(IReadOnlyList<LogLine> lines) => dispatcher.TryEnqueue(() =>
    {
        Log.Append(lines);
        LogAppended?.Invoke(lines);
    });

    public void PostNotification(UserNotification notification) => dispatcher.TryEnqueue(() =>
        notify(notification.Identifier(), notification.Title(), notification.Body()));

    private void RestartClock()
    {
        Now = DateTimeOffset.Now;
        ticker.Stop();
        ticker.Interval = Counting ? TimeSpan.FromSeconds(1) : TimeSpan.FromSeconds(30);
        ticker.Start();
    }

    private bool Counting => Connection?.NextAttempt() is { } next && next > DateTimeOffset.Now;

    private void Tick()
    {
        Now = DateTimeOffset.Now;
        if (!Counting && ticker.Interval < TimeSpan.FromSeconds(30))
        {
            ticker.Interval = TimeSpan.FromSeconds(30);
        }
        Changed?.Invoke();
    }
}
