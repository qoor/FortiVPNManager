using System.Diagnostics;
using Xunit;

namespace FortiVPNManager.OsIntegration.Tests;

public class UpdateWaiterTests
{
    private const string App = @"C:\Program Files\FortiVPNManager\FortiVPNManager.exe";
    private const string CharMap = @"C:\Windows\System32\charmap.exe";
    private static readonly TimeSpan s_limit = TimeSpan.FromSeconds(30);

    [Fact]
    public void TheWaiterIsAHiddenCmdThatReadsTheInstallerFromItsInput()
    {
        ProcessStartInfo start = UpdateWaiter.StartInfo(1111, App, "FortiVPNManager.exe");
        Assert.Equal(@"C:\Windows\System32\cmd.exe", start.FileName);
        Assert.Empty(start.ArgumentList);
        Assert.False(start.UseShellExecute);
        Assert.True(start.CreateNoWindow);
        Assert.True(start.RedirectStandardInput);
        Assert.False(start.RedirectStandardOutput);
        Assert.Equal(Environment.SystemDirectory, start.WorkingDirectory);
    }

    [Fact]
    public void TheWaiterChecksEachProcessByIdAndProgramThenStartsTheApp()
    {
        string arguments = UpdateWaiter.StartInfo(1111, App, "FortiVPNManager.exe").Arguments;
        Assert.Equal(
            """"/d /q /v:on /c set "fvm_installer=" & (set /p fvm_installer=)& if defined fvm_installer for /l %i in (0,0,1) do @((tasklist /nh /fo csv /fi "PID eq 1111" /fi "IMAGENAME eq FortiVPNManager.exe" | find """1111""" >nul || tasklist /nh /fo csv /fi "PID eq !fvm_installer!" /fi "IMAGENAME eq msiexec.exe" | find """!fvm_installer!""" >nul || (start "" "C:\Program Files\FortiVPNManager\FortiVPNManager.exe" & exit)) & ping -n 2 127.0.0.1 >nul)"""",
            arguments);
        Assert.All(arguments, character => Assert.True(character < 128, $"U+{(int)character:X4} is not ASCII"));
    }

    [Fact]
    public void TheAppStartsOnlyAfterItAndTheInstallerEnded()
    {
        using Process app = Process.Start(CharMap)!;
        using Process installer = Process.Start(new ProcessStartInfo(@"C:\Windows\System32\msiexec.exe", "/?"))!;
        List<int> before = CharMaps();
        using Process waiter = Process.Start(UpdateWaiter.StartInfo(app.Id, CharMap, "charmap.exe"))!;
        try
        {
            Give(waiter, installer.Id);
            Thread.Sleep(TimeSpan.FromSeconds(2));
            Assert.False(waiter.HasExited, "the waiter ended while the app ran");
            Assert.Empty(CharMaps().Except(before));

            app.Kill();
            Thread.Sleep(TimeSpan.FromSeconds(3));
            Assert.False(waiter.HasExited, "the waiter ended while the installer ran");
            Assert.Empty(CharMaps().Except(before));

            installer.Kill();
            Assert.True(waiter.WaitForExit(TimeSpan.FromSeconds(5)), "the waiter did not end after the installer");
            Assert.Single(Until(() => CharMaps().Except(before).ToList(), started => started.Count > 0));
        }
        finally
        {
            End(waiter, app, installer);
            EndNewCharMaps(before);
        }
    }

    [Fact]
    public void AnotherProgramUnderTheIdsCountsAsEnded()
    {
        using Process notApp = Ping();
        using Process notInstaller = Ping();
        List<int> before = CharMaps();
        using Process waiter = Process.Start(UpdateWaiter.StartInfo(notApp.Id, CharMap, "charmap.exe"))!;
        try
        {
            Give(waiter, notInstaller.Id);
            Assert.True(waiter.WaitForExit(TimeSpan.FromSeconds(5)), "the waiter waited for programs that are not the app or the installer");
            Assert.False(notApp.HasExited);
            Assert.False(notInstaller.HasExited);
            Assert.Single(Until(() => CharMaps().Except(before).ToList(), started => started.Count > 0));
        }
        finally
        {
            End(waiter, notApp, notInstaller);
            EndNewCharMaps(before);
        }
    }

    [Fact]
    public void AWaiterGivenNoInstallerEndsWithoutStartingTheApp()
    {
        List<int> before = CharMaps();
        using Process waiter = Process.Start(UpdateWaiter.StartInfo(Environment.ProcessId, CharMap, "charmap.exe"))!;
        try
        {
            waiter.StandardInput.Close();
            Assert.True(waiter.WaitForExit(TimeSpan.FromSeconds(5)), "the waiter waited without an installer");
            Thread.Sleep(TimeSpan.FromSeconds(1));
            Assert.Empty(CharMaps().Except(before));
        }
        finally
        {
            End(waiter);
            EndNewCharMaps(before);
        }
    }

    private static void Give(Process waiter, int installer)
    {
        waiter.StandardInput.WriteLine(installer);
        waiter.StandardInput.Close();
    }

    private static Process Ping() =>
        Process.Start(new ProcessStartInfo(@"C:\Windows\System32\PING.EXE", "-n 60 127.0.0.1") { CreateNoWindow = true, UseShellExecute = false })!;

    private static List<int> CharMaps()
    {
        Process[] processes = Process.GetProcessesByName("charmap");
        var ids = processes.Select(process => process.Id).ToList();
        foreach (Process process in processes)
        {
            process.Dispose();
        }
        return ids;
    }

    private static T Until<T>(Func<T> read, Func<T, bool> done)
    {
        DateTime deadline = DateTime.UtcNow + s_limit;
        while (true)
        {
            T? value = read();
            if (done(value))
            {
                return value;
            }
            Assert.True(DateTime.UtcNow < deadline, "nothing changed within the limit");
            Thread.Sleep(200);
        }
    }

    private static void End(params Process[] processes)
    {
        foreach (Process process in processes)
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
    }

    private static void EndNewCharMaps(List<int> before)
    {
        foreach (int id in CharMaps().Except(before))
        {
            try
            {
                using var process = Process.GetProcessById(id);
                process.Kill();
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException)
            {
            }
        }
    }
}
