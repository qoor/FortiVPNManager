using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using FortiVPNManager.Contract.AppInternal;
using Microsoft.Win32;

namespace FortiVPNManager.OsIntegration;

/// <summary>Why the app quits at launch without the Helper.</summary>
public enum HelperStartFailure
{
    NotInstalled,
    Disabled,

    /// <summary>This account may not start the service, or not even read its state.</summary>
    NoPermission,

    /// <summary>Windows gave up waiting for the Helper's process to take the start.</summary>
    TimedOut,

    /// <summary>The Helper stopped with an error while it started, or Windows could not start it.</summary>
    ErrorWhileStarting,
}

/// <summary>How the wait for the Helper at launch ended.</summary>
public abstract record HelperStartResult
{
    private HelperStartResult() { }

    public sealed record Running : HelperStartResult;

    /// <summary>Windows signs out or shuts down; the app quits without a word.</summary>
    public sealed record SessionEnding : HelperStartResult;

    /// <param name="Detail">What happened, in English for the event log.</param>
    public sealed record Failed(HelperStartFailure Reason, string Detail) : HelperStartResult;
}

/// <summary>The Helper's service as the app sees it, with the rights every account has.</summary>
internal interface IHelperService
{
    HelperServiceState State();

    /// <summary>Asks the service manager to start the service, as Start in the Services console does.</summary>
    StartAnswer Start();

    /// <summary>Whether a Helper process is there.</summary>
    bool ProcessLeft();
}

internal enum HelperServiceState
{
    NotInstalled,
    NoPermission,
    Stopped,

    /// <summary>Starting, stopping, or anything else on the way to running or stopped.</summary>
    Changing,
    Running,
}

/// <summary>The service manager's answer to a start.</summary>
internal abstract record StartAnswer
{
    private StartAnswer() { }

    /// <summary>The service starts, or ran already.</summary>
    public sealed record Requested : StartAnswer;

    /// <summary>No later start can do better.</summary>
    public sealed record Refused(HelperStartFailure Reason) : StartAnswer;

    /// <summary>Windows gave up waiting for the service's process, after 30 s by default.</summary>
    public sealed record TimedOut : StartAnswer;

    public sealed record Failed(int Error) : StartAnswer;
}

/// <summary>
/// The app is no use without the Helper, so at launch, before anything else starts, it makes sure
/// the Helper's service runs. The installer registers the service to start at boot and lets every
/// account start it, but leaves starting it to the app. A service that is not there, is disabled or
/// may not be started by this account fails at once. One that is stopped is started, and one on its
/// way is waited for, however long its start takes. A start that fails is made once more, as the
/// macOS app tries a Helper call once more; when that one fails too, the app quits.
/// </summary>
public static class HelperStart
{
    /// <summary>The app's starts of the service: the first and the one more.</summary>
    private const int Starts = 2;

    private static readonly TimeSpan s_poll = TimeSpan.FromSeconds(1);

    /// <summary>Waits on the calling thread. A failed start and a failure are written to the event log.</summary>
    public static HelperStartResult Run()
    {
        using var ending = new ManualResetEventSlim();
        void onEnding(object _1, SessionEndingEventArgs _2) => ending.Set();
        // From a pool thread, as in SystemEvents.Start: the first use of them decides their thread.
        Task.Run(() => Microsoft.Win32.SystemEvents.SessionEnding += onEnding).Wait();
        try
        {
            using var service = new WindowsHelperService();
            HelperStartResult result = Wait(
                service,
                interval => !ending.Wait(interval),
                line => SystemLog.Write(LogLevel.Notice, LogCategory.Readiness, line));
            if (result is HelperStartResult.Failed failed)
            {
                SystemLog.Write(LogLevel.Error, LogCategory.Readiness, failed.Detail);
            }
            return result;
        }
        finally
        {
            Microsoft.Win32.SystemEvents.SessionEnding -= onEnding;
        }
    }

    /// <param name="pause">Waits that long; false when the session ends meanwhile.</param>
    /// <param name="retrying">
    /// Takes the event log line for the first start's failure, as soon as it is seen. The second start
    /// follows the same steps as the first, unless Windows' own restart of the service makes it run first.
    /// </param>
    internal static HelperStartResult Wait(IHelperService service, Func<TimeSpan, bool> pause, Action<string> retrying)
    {
        int starts = 0;
        bool watching = false;
        bool announced = false;
        (HelperStartFailure Reason, string Why)? failure = null;
        while (true)
        {
            switch (service.State())
            {
                case HelperServiceState.Running:
                    return new HelperStartResult.Running();
                case HelperServiceState.NotInstalled:
                    return Refused(HelperStartFailure.NotInstalled, "the Helper service is not installed");
                case HelperServiceState.NoPermission:
                    return Refused(HelperStartFailure.NoPermission, "this account may not read the Helper service's state");
                case HelperServiceState.Stopped:
                    if (watching)
                    {
                        watching = false;
                        failure = (HelperStartFailure.ErrorWhileStarting, "it stopped with an error while starting; its own event log entries say why");
                    }
                    if (failure is { } last && starts == Starts)
                    {
                        return new HelperStartResult.Failed(
                            last.Reason,
                            $"An error occurred while the Helper was starting ({last.Why}); FortiVPNManager quits (attempt {starts}/{Starts})");
                    }
                    // A start that timed out leaves its process behind until it exits; the next
                    // start waits for it to go.
                    if (service.ProcessLeft())
                    {
                        break;
                    }
                    starts++;
                    switch (service.Start())
                    {
                        case StartAnswer.Requested:
                            watching = true;
                            break;
                        case StartAnswer.Refused refused:
                            return Refused(refused.Reason, refused.Reason switch
                            {
                                HelperStartFailure.NotInstalled => "the Helper service is not installed",
                                HelperStartFailure.Disabled => "the Helper service is disabled",
                                HelperStartFailure.NoPermission => "this account may not start the Helper service",
                                HelperStartFailure.TimedOut or HelperStartFailure.ErrorWhileStarting =>
                                    throw new InvalidOperationException($"a start is never refused for {refused.Reason}"),
                                _ => throw new InvalidOperationException($"unknown start refusal {refused.Reason}"),
                            });
                        case StartAnswer.TimedOut:
                            failure = (HelperStartFailure.TimedOut, "it did not start in time: Windows gave up waiting for its process");
                            break;
                        case StartAnswer.Failed failed:
                            failure = (HelperStartFailure.ErrorWhileStarting, $"Windows could not start its service: error {failed.Error}");
                            break;
                        default:
                            throw new InvalidOperationException("unknown start answer");
                    }
                    break;
                case HelperServiceState.Changing:
                    break;
                default:
                    throw new InvalidOperationException("unknown Helper service state");
            }
            if (failure is { } first && starts < Starts && !announced)
            {
                announced = true;
                retrying($"An error occurred while the Helper was starting ({first.Why}); starting the Helper again (attempt {starts}/{Starts})");
            }
            if (!pause(s_poll))
            {
                return new HelperStartResult.SessionEnding();
            }
        }
    }

    /// <summary>A failure no second start can help.</summary>
    private static HelperStartResult.Failed Refused(HelperStartFailure reason, string why) =>
        new(reason, $"The Helper failed to start ({why}); FortiVPNManager quits");
}

/// <summary>
/// The Helper's service through the service manager, opened only for what the installer lets
/// every account do: read its state and start it.
/// </summary>
internal sealed class WindowsHelperService : IHelperService, IDisposable
{
    /// <summary>The Helper's executable without ".exe" (components/helper/windows/Cargo.toml).</summary>
    private const string HelperProcess = "fvm-helper";

    private const uint ManagerConnect = 0x0001;
    private const uint QueryStatusAccess = 0x0004;
    private const uint StartAccess = 0x0010;
    private const uint Stopped = 1;
    private const uint Running = 4;
    private const int AccessDenied = 5;
    private const int RequestTimedOut = 1053;
    private const int AlreadyRunning = 1056;
    private const int ServiceDisabled = 1058;
    private const int ServiceMissing = 1060;
    private const int MarkedForDelete = 1072;

    [StructLayout(LayoutKind.Sequential)]
    private struct ServiceStatus
    {
        public uint ServiceType;
        public uint CurrentState;
        public uint ControlsAccepted;
        public uint Win32ExitCode;
        public uint ServiceSpecificExitCode;
        public uint CheckPoint;
        public uint WaitHint;
    }

    private readonly nint _manager;

    public WindowsHelperService()
    {
        _manager = OpenSCManagerW(null, null, ManagerConnect);
        if (_manager == 0)
        {
            throw new Win32Exception();
        }
    }

    public HelperServiceState State()
    {
        nint service = OpenServiceW(_manager, ServiceRegistration.HelperService, QueryStatusAccess);
        if (service == 0)
        {
            return Marshal.GetLastWin32Error() switch
            {
                ServiceMissing => HelperServiceState.NotInstalled,
                AccessDenied => HelperServiceState.NoPermission,
                var error => throw new Win32Exception(error),
            };
        }
        try
        {
            return !QueryServiceStatus(service, out ServiceStatus status)
                ? throw new Win32Exception()
                : status.CurrentState switch
                {
                    Stopped => HelperServiceState.Stopped,
                    Running => HelperServiceState.Running,
                    _ => HelperServiceState.Changing,
                };
        }
        finally
        {
            _ = CloseServiceHandle(service);
        }
    }

    public StartAnswer Start()
    {
        nint service = OpenServiceW(_manager, ServiceRegistration.HelperService, StartAccess);
        if (service == 0)
        {
            return Marshal.GetLastWin32Error() switch
            {
                ServiceMissing => new StartAnswer.Refused(HelperStartFailure.NotInstalled),
                AccessDenied => new StartAnswer.Refused(HelperStartFailure.NoPermission),
                var error => throw new Win32Exception(error),
            };
        }
        try
        {
            return StartServiceW(service, 0, 0)
                ? new StartAnswer.Requested()
                : Marshal.GetLastWin32Error() switch
                {
                    AlreadyRunning => new StartAnswer.Requested(),
                    ServiceDisabled => new StartAnswer.Refused(HelperStartFailure.Disabled),
                    // Being uninstalled.
                    MarkedForDelete => new StartAnswer.Refused(HelperStartFailure.NotInstalled),
                    RequestTimedOut => new StartAnswer.TimedOut(),
                    var error => new StartAnswer.Failed(error),
                };
        }
        finally
        {
            _ = CloseServiceHandle(service);
        }
    }

    /// <summary>
    /// By name, from the process list every account can read: the Helper runs as the system account,
    /// whose processes another account cannot open to wait on.
    /// </summary>
    public bool ProcessLeft()
    {
        Process[] processes = Process.GetProcessesByName(HelperProcess);
        foreach (Process process in processes)
        {
            process.Dispose();
        }
        return processes.Length > 0;
    }

    public void Dispose() => CloseServiceHandle(_manager);

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern nint OpenSCManagerW(string? machine, string? database, uint access);

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern nint OpenServiceW(nint manager, string name, uint access);

    [DllImport("advapi32.dll", SetLastError = true, ExactSpelling = true)]
    private static extern bool QueryServiceStatus(nint service, out ServiceStatus status);

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern bool StartServiceW(nint service, uint argumentCount, nint arguments);

    [DllImport("advapi32.dll", SetLastError = true, ExactSpelling = true)]
    private static extern bool CloseServiceHandle(nint handle);
}
