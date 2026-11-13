using System.ComponentModel;
using System.Diagnostics;
using System.Security;
using System.Text.Json;
using FortiVPNManager.Contract;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The Windows event log, in macOS unified log's place, under the runtime's categories. The install
/// script registers the source, which takes administrator rights; until then a line is dropped.
/// </summary>
internal static class SystemLog
{
    public const string Source = "FortiVPNManager";

    /// <summary>
    /// An entry takes at most 31,839 characters, so a longer line goes in parts this long, each an
    /// entry of its own, in order.
    /// </summary>
    internal const int MaxPart = 30_000;

    public static void Write(LogLevel level, LogCategory category, string message)
    {
        var type = level == LogLevel.Error ? EventLogEntryType.Error : EventLogEntryType.Information;
        var name = Name(category);
        var parts = Parts(message);
        try
        {
            if (parts.Count == 1)
            {
                EventLog.WriteEntry(Source, $"{name}: {message}", type);
                return;
            }
            for (var index = 0; index < parts.Count; index++)
            {
                EventLog.WriteEntry(Source, $"{name} ({index + 1}/{parts.Count}): {parts[index]}", type);
            }
        }
        catch (Exception error) when (error is InvalidOperationException or SecurityException or Win32Exception)
        {
            // The source is not registered yet.
        }
    }

    /// <summary>
    /// <paramref name="message"/> in parts of at most <see cref="MaxPart"/> characters. A character
    /// that takes two UTF-16 units, such as an emoji, stays whole.
    /// </summary>
    internal static List<string> Parts(string message)
    {
        var parts = new List<string>();
        var start = 0;
        do
        {
            var end = Math.Min(start + MaxPart, message.Length);
            if (end < message.Length && char.IsHighSurrogate(message[end - 1]))
            {
                end--;
            }
            parts.Add(message[start..end]);
            start = end;
        }
        while (start < message.Length);
        return parts;
    }

    /// <summary>The category's name as on macOS, such as "bypass-routes".</summary>
    private static string Name(LogCategory category) =>
        JsonSerializer.Serialize(category, MessageCoding.Options).Trim('"');
}
