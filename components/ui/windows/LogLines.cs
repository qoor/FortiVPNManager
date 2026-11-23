using System.Globalization;
using System.Text;
using FortiVPNManager.Contract.AppInternal;

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

    private readonly List<LogEntry> _entries = [];
    private ulong _nextId = 1;
    private int _byteCount;

    public IReadOnlyList<LogEntry> Entries => _entries;

    public void Append(IReadOnlyList<LogLine> lines)
    {
        ArgumentNullException.ThrowIfNull(lines);
        foreach (LogLine line in lines)
        {
            _entries.Add(new LogEntry(_nextId++, line));
            _byteCount += Cost(line);
        }
        int drop = 0;
        while (_entries.Count - drop > MaxLines || (_byteCount > MaxBytes && drop < _entries.Count))
        {
            _byteCount -= Cost(_entries[drop].Line);
            drop++;
        }
        _entries.RemoveRange(0, drop);
    }

    /// <summary>Entries newer than <paramref name="id"/> (all of them when it is null or older than the buffer).</summary>
    public IEnumerable<LogEntry> After(ulong? id) => id is { } last ? _entries.Where(entry => entry.Id > last) : _entries;

    /// <summary>"Clear": empties the pane only.</summary>
    public void Clear()
    {
        _entries.Clear();
        _byteCount = 0;
    }

    /// <summary>"Copy All": the lines in the log file's format, times in <paramref name="zone"/>.</summary>
    public string CopyAllText(TimeZoneInfo zone) =>
        string.Join("\n", _entries.Select((entry, index) => FileLine(entry.Line, zone, afterAnother: index > 0)));

    /// <summary>
    /// <c>&lt;ISO 8601 local time with milliseconds&gt; [info|err|▸] &lt;text&gt;</c>, the line format of the
    /// log file, such as <c>2026-10-15T14:03:22.123+09:00 [info] Connected to gateway.</c> Universal
    /// time ends with Z, as the file writes it.
    /// </summary>
    public static string FileLine(LogLine line, TimeZoneInfo zone, bool afterAnother)
    {
        ArgumentNullException.ThrowIfNull(line);
        DateTimeOffset local = TimeZoneInfo.ConvertTime(line.Time, zone);
        string offset = local.Offset == TimeSpan.Zero
            ? "Z"
            : (local.Offset < TimeSpan.Zero ? "-" : "+") + local.Offset.ToString("hh\\:mm", CultureInfo.InvariantCulture);
        return $"{Gap(line, afterAnother)}{local.ToString("yyyy-MM-dd'T'HH:mm:ss.fff", CultureInfo.InvariantCulture)}{offset} [{Tag(line.Source)}] {line.Text}";
    }

    /// <summary>
    /// The two empty lines before the first line of an attempt, unless no line comes before it. The Log
    /// pane puts the same before its own line.
    /// </summary>
    public static string Gap(LogLine line, bool afterAnother)
    {
        ArgumentNullException.ThrowIfNull(line);
        return line.StartsAttempt && afterAnother ? "\n\n" : "";
    }

    public static string Tag(LogSource source) => source switch
    {
        LogSource.Info => "info",
        LogSource.Error => "err",
        LogSource.Note => "▸",
        _ => throw new InvalidOperationException($"unknown log source {source}"),
    };

    private static int Cost(LogLine line) => Encoding.UTF8.GetByteCount(line.Text) + 1;
}
