using System.Buffers.Binary;
using System.Text.Json;

namespace FortiVPNManager.Contract;

/// <summary>
/// The framing of every message on the runtime's pipes and the Helper's pipe: a 4-byte big-endian
/// length and that many bytes of UTF-8 JSON (components/contract/src/stream.rs), coded by
/// <see cref="MessageCoding"/>. Each direction takes its own message type, so a case is always
/// written inside its type's wrapper. A stream is written by one thread at a time: two writers at
/// once can interleave their frames.
/// </summary>
public static class MessageFraming
{
    /// <summary>Writes one message to the runtime and flushes it.</summary>
    public static void Write(Stream stream, AppInternal.ToRuntime message) => WriteFrame(stream, message);

    /// <summary>Writes one request to the Helper and flushes it.</summary>
    public static void Write(Stream stream, HelperPipe.Request request) => WriteFrame(stream, request);

    /// <summary>Blocks until a whole message from the runtime arrives; see <see cref="ReadFrame{T}"/>.</summary>
    public static AppInternal.FromRuntime ReadFromRuntime(Stream stream) => ReadFrame<AppInternal.FromRuntime>(stream);

    /// <summary>Blocks until a whole message from the Helper arrives; see <see cref="ReadFrame{T}"/>.</summary>
    public static HelperPipe.Message ReadFromHelper(Stream stream) => ReadFrame<HelperPipe.Message>(stream);

    private static void WriteFrame<T>(Stream stream, T message)
    {
        var payload = JsonSerializer.SerializeToUtf8Bytes(message, MessageCoding.Options);
        var frame = new byte[4 + payload.Length];
        BinaryPrimitives.WriteUInt32BigEndian(frame, (uint)payload.Length);
        payload.CopyTo(frame, 4);
        stream.Write(frame);
        stream.Flush();
    }

    /// <summary>
    /// The end of the stream, even within a message, throws <see cref="EndOfStreamException"/>; a
    /// message that does not decode throws <see cref="JsonException"/>.
    /// </summary>
    private static T ReadFrame<T>(Stream stream)
    {
        Span<byte> header = stackalloc byte[4];
        stream.ReadExactly(header);
        var payload = new byte[BinaryPrimitives.ReadUInt32BigEndian(header)];
        stream.ReadExactly(payload);
        return JsonSerializer.Deserialize<T>(payload, MessageCoding.Options)
            ?? throw new JsonException("a message is never null");
    }
}
