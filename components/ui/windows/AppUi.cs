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
    private readonly DispatcherQueue _dispatcher;
    private readonly Action<string, string, string, string?> _notify;
    private readonly DispatcherQueueTimer _ticker;
    private SettingsWindow? _settings;

    /// <param name="send">Sends a command to the runtime.</param>
    /// <param name="notify">Posts a notification as (identifier, title, body, button), replacing one of the same identifier.</param>
    public AppUi(DispatcherQueue dispatcher, Action<Command> send, Action<string, string, string, string?> notify)
    {
        ArgumentNullException.ThrowIfNull(dispatcher);
        this._dispatcher = dispatcher;
        this._notify = notify;
        Send = send;
        _ticker = dispatcher.CreateTimer();
        _ticker.Tick += (_, _) => Tick();
    }

    public Action<Command> Send { get; }

    public ConnectionSection? Connection { get; private set; }

    public ScheduleSection? Schedule { get; private set; }

    public ReadinessSection? Readiness { get; private set; }

    public GmailSection? Gmail { get; private set; }

    public BypassRulesSection? BypassRules { get; private set; }

    public ConfigSection? Config { get; private set; }

    public AutoUpdateSection? AutoUpdate { get; private set; }

    /// <summary>The lines the Log pane shows; the log file keeps them all.</summary>
    public LogLines Log { get; } = new();

    public AccountDraft AccountDraft { get; } = new();

    /// <summary>The Account pane's disclosures the person opened, kept while the app runs (macOS SettingsNavigation).</summary>
    public HashSet<string> OpenDisclosures { get; } = [];

    /// <summary>Whether the panes show the rows of a setting shown only in some states; the search lists only those it can go to.</summary>
    public bool IsShown(SearchCondition condition) => condition switch
    {
        SearchCondition.Connected => Connection?.Status is ConnectionStatus.Connected,
        SearchCondition.GmailApi => Gmail?.Method == OtpMethod.GmailApi,
        SearchCondition.VpnBackendFound => Readiness?.Backend is not null,
        SearchCondition.RegisterHelper => Readiness is { RegisterHelper: not ButtonState.Hidden },
        SearchCondition.AddCurrentNetwork => BypassRules is { AddCurrentNetwork: not ButtonState.Hidden },
        _ => throw new InvalidOperationException($"unknown condition {condition}"),
    };

    /// <summary>The Log pane's Auto-scroll, kept while the app runs.</summary>
    public bool LogAutoScroll { get; set; } = true;

    /// <summary>"Now" for relative times: set by every connection section, every 30 s, and every second of a countdown.</summary>
    public DateTimeOffset Now { get; private set; } = DateTimeOffset.Now;

    public TimeStyle Time => TimeStyle.Current(Now);

    /// <summary>A section changed or the clock ticked.</summary>
    public event Action? Changed;

    /// <summary>Lines were added to the log.</summary>
    public event Action<IReadOnlyList<LogLine>>? LogAppended;

    /// <summary>The settings window, made when it is needed while none is open; closing releases it.</summary>
    public SettingsWindow Settings
    {
        get
        {
            if (_settings is null)
            {
                _settings = new SettingsWindow(this);
                _settings.Closed += (_, _) => _settings = null;
            }
            return _settings;
        }
    }

    /// <summary>Closes the settings window when it is open; quitting doesn't make one only to close it.</summary>
    public void CloseSettings() => _settings?.Close();

    public void OpenNotification(string? identifier, bool button)
    {
        if (identifier == BypassText.UpdateAvailableIdentifier)
        {
            Settings.ShowUpdates();
            if (button)
            {
                Send(new Command.UpdateNowFromNotification());
            }
        }
        else
        {
            Settings.Show(SettingsPane.ExceptionRules);
        }
    }

    /// <summary>Sends what Settings still has in its fields; quitting asks the runtime only after it.</summary>
    public void FinishSettingsTyping() => _settings?.FinishTyping();

    // From the runtime, on its reader thread.

    public void Apply(Section section) => _dispatcher.TryEnqueue(() =>
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
            case Section.AutoUpdate update:
                AutoUpdate = update.Value;
                break;
            default:
                throw new InvalidOperationException($"unknown section {section}");
        }
        Changed?.Invoke();
    });

    public void AppendLog(IReadOnlyList<LogLine> lines) => _dispatcher.TryEnqueue(() =>
    {
        Log.Append(lines);
        LogAppended?.Invoke(lines);
    });

    public void PostNotification(UserNotification notification) => _dispatcher.TryEnqueue(() =>
        _notify(notification.Identifier(), notification.Title(), notification.Body(), notification.ButtonTitle()));

    private void RestartClock()
    {
        Now = DateTimeOffset.Now;
        _ticker.Stop();
        _ticker.Interval = Counting ? TimeSpan.FromSeconds(1) : TimeSpan.FromSeconds(30);
        _ticker.Start();
    }

    private bool Counting => Connection?.NextAttempt() is { } next && next > DateTimeOffset.Now;

    private void Tick()
    {
        Now = DateTimeOffset.Now;
        if (!Counting && _ticker.Interval < TimeSpan.FromSeconds(30))
        {
            _ticker.Interval = TimeSpan.FromSeconds(30);
        }
        Changed?.Invoke();
    }
}
