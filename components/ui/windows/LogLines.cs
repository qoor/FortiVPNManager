using System.Globalization;
using System.Text;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui;

/// <summary>One line of the Log pane. <c>Id</c> is local, strictly increasing and never reused.</summary>
public sealed record LogEntry(ulong Id, LogLine Line);

/// <summary>
/// The lines the Log pane shows, oldest first (components/ui/macos/LogLines.swift): the newest ones
/// the runtime sent, within a line and a size limit. The log file keeps everything; that is the runtime's.
/// </summary>
public sealed class LogLines
{
    public const int MaxLines = 5_000;
    public const int MaxBytes = 2 * 1024 * 1024;

    private readonly List<LogEntry> entries = [];
    private ulong nextId = 1;
    private int byteCount;

    public IReadOnlyList<LogEntry> Entries => entries;

    public void Append(IReadOnlyList<LogLine> lines)
    {
        foreach (var line in lines)
        {
            entries.Add(new LogEntry(nextId++, line));
            byteCount += Cost(line);
        }
        var drop = 0;
        while (entries.Count - drop > MaxLines || (byteCount > MaxBytes && drop < entries.Count))
        {
            byteCount -= Cost(entries[drop].Line);
            drop++;
        }
        entries.RemoveRange(0, drop);
    }

    /// <summary>Entries newer than <paramref name="id"/> (all of them when it is null or older than the buffer).</summary>
    public IEnumerable<LogEntry> After(ulong? id) => id is { } last ? entries.Where(entry => entry.Id > last) : entries;

    /// <summary>"Clear": empties the pane only.</summary>
    public void Clear()
    {
        entries.Clear();
        byteCount = 0;
    }

    /// <summary>"Copy All": the lines in the log file's format, times in <paramref name="zone"/>.</summary>
    public string CopyAllText(TimeZoneInfo zone) => string.Join("\n", entries.Select(entry => FileLine(entry.Line, zone)));

    /// <summary>
    /// <c>&lt;ISO 8601 local time with milliseconds&gt; [out|err|▸] &lt;text&gt;</c>, the line format of the
    /// log file, such as <c>2026-10-15T14:03:22.123+09:00 [out] Connected to gateway.</c> Universal
    /// time ends with Z, as the file writes it.
    /// </summary>
    public static string FileLine(LogLine line, TimeZoneInfo zone)
    {
        var local = TimeZoneInfo.ConvertTime(line.Time, zone);
        var offset = local.Offset == TimeSpan.Zero
            ? "Z"
            : (local.Offset < TimeSpan.Zero ? "-" : "+") + local.Offset.ToString("hh\\:mm", CultureInfo.InvariantCulture);
        return $"{local.ToString("yyyy-MM-dd'T'HH:mm:ss.fff", CultureInfo.InvariantCulture)}{offset} [{Tag(line.Source)}] {line.Text}";
    }

    public static string Tag(LogSource source) => source switch
    {
        LogSource.Stdout => "out",
        LogSource.Stderr => "err",
        LogSource.Note => "▸",
        _ => throw new InvalidOperationException($"unknown log source {source}"),
    };

    private static int Cost(LogLine line) => Encoding.UTF8.GetByteCount(line.Text) + 1;
}
