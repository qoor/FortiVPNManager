using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using FortiVPNManager.Contract;
using Microsoft.Win32.SafeHandles;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The runtime (fvm_runtime.dll, next to the app) in this process, reached only through the two
/// pipes its start function returns: one thread reads the runtime's messages and one writes the
/// app's, so neither waits on the caller. Neither side closes its end on purpose, so a failed read
/// or write, a message that does not decode and the end of a pipe all stop the process, as on
/// macOS.
/// </summary>
public sealed class RuntimeLink
{
    /// <summary>The app's ends of the runtime's pipes (components/runtime/src/windows.rs).</summary>
    [StructLayout(LayoutKind.Sequential)]
    private struct AppPipes
    {
        public nuint Read;
        public nuint Write;
    }

    [DllImport("fvm_runtime", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    private static extern AppPipes fvm_runtime_start();

    private readonly BlockingCollection<AppInternal.ToRuntime> outgoing = new();

    private RuntimeLink() { }

    /// <summary>
    /// Starts the runtime; call once. <paramref name="onMessage"/> runs on the reader thread for each
    /// message, one at a time in arrival order.
    /// </summary>
    public static RuntimeLink Start(Action<AppInternal.FromRuntime> onMessage)
    {
        var pipes = fvm_runtime_start();
        var fromRuntime = Open(pipes.Read, FileAccess.Read);
        var toRuntime = Open(pipes.Write, FileAccess.Write);
        var link = new RuntimeLink();
        StartThread("fvm-runtime-read", () =>
        {
            while (true)
            {
                AppInternal.FromRuntime message;
                try
                {
                    message = MessageFraming.ReadFromRuntime(fromRuntime);
                }
                catch (Exception error)
                {
                    Environment.FailFast($"cannot read a message from the runtime: {error.Message}");
                    return;
                }
                onMessage(message);
            }
        });
        StartThread("fvm-runtime-write", () =>
        {
            foreach (var message in link.outgoing.GetConsumingEnumerable())
            {
                try
                {
                    MessageFraming.Write(toRuntime, message);
                }
                catch (Exception error)
                {
                    Environment.FailFast($"cannot write a message to the runtime: {error.Message}");
                }
            }
        });
        return link;
    }

    /// <summary>Messages go out in call order, from any thread.</summary>
    public void Send(AppInternal.ToRuntime message) => outgoing.Add(message);

    private static FileStream Open(nuint handle, FileAccess access) =>
        new(new SafeFileHandle((nint)handle, ownsHandle: true), access, bufferSize: 0);

    private static void StartThread(string name, Action body) =>
        new Thread(() => body()) { Name = name, IsBackground = true }.Start();
}
