using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.HelperPipe;

/// <summary>
/// Messages over the Windows Helper service's named pipe (components/contract/src/helper_pipe.rs),
/// in XPC's place on macOS, framed by <see cref="MessageFraming"/>. The app numbers its requests;
/// the Helper answers each one once with that number, and sends a run's events and output in the
/// order they happened on the same pipe. The JSON payloads are passed to and from the runtime
/// unread.
/// </summary>
public static class Pipe
{
    /// <summary>The pipe's name as a client opens it on this computer. The system account owns it.</summary>
    public const string Name = "com.qoor.FortiVPNManager.Helper";
}

/// <summary>Sent by the app. <c>Id</c> names the request in the Helper's answer.</summary>
[JsonConverter(typeof(CasesConverter<Request>))]
[JsonCase(typeof(RequestBuild), "build")]
[JsonCase(typeof(RequestConnect), "connect")]
[JsonCase(typeof(RequestSubmitOtp), "submitOtp")]
[JsonCase(typeof(RequestDisconnect), "disconnect")]
[JsonCase(typeof(RequestStatus), "status")]
[JsonCase(typeof(RequestExitWhenIdle), "exitWhenIdle")]
[JsonCase(typeof(RequestReadBypassRoutes), "readBypassRoutes")]
[JsonCase(typeof(RequestWriteBypassRoutes), "writeBypassRoutes")]
[JsonCase(typeof(RequestPrepareUpdate), "prepareUpdate")]
[JsonCase(typeof(RequestInstallUpdate), "installUpdate")]
[JsonCase(typeof(RequestReadPreparedUpdate), "readPreparedUpdate")]
[JsonCase(typeof(RequestDiscardPreparedUpdate), "discardPreparedUpdate")]
public abstract record Request
{
    private protected Request() { }
}

/// <summary>
/// The build check, answered by <see cref="MessageBuild"/>; <c>App</c> is the app's build.
/// Goes between builds, so its shape and its answer's never change.
/// </summary>
public sealed record RequestBuild(ulong Id, string App) : Request;

/// <summary><c>Json</c> is a run request. Answered by <see cref="MessageReply"/>.</summary>
public sealed record RequestConnect(ulong Id, string Json) : Request
{
    public override string ToString() => $"Connect {{ Id = {Id}, Json = <redacted> }}";
}

public sealed record RequestSubmitOtp(ulong Id, string Attempt, string Code) : Request
{
    public override string ToString() => $"SubmitOtp {{ Id = {Id}, Attempt = {Attempt}, Code = <redacted> }}";
}

public sealed record RequestDisconnect(ulong Id, string Attempt, string Reason) : Request;

public sealed record RequestStatus(ulong Id) : Request;

/// <summary>
/// Answered by <see cref="MessageReplyFlag"/>; the Windows Helper always answers false. Goes
/// between builds, so its shape and its answer's never change.
/// </summary>
public sealed record RequestExitWhenIdle(ulong Id) : Request;

public sealed record RequestReadBypassRoutes(ulong Id) : Request;

public sealed record RequestWriteBypassRoutes(ulong Id, string Json) : Request;

public sealed record RequestPrepareUpdate(ulong Id, string Json) : Request;

public sealed record RequestInstallUpdate(ulong Id, string Json) : Request;

public sealed record RequestReadPreparedUpdate(ulong Id) : Request;

public sealed record RequestDiscardPreparedUpdate(ulong Id) : Request;

/// <summary>Sent by the Helper.</summary>
[JsonConverter(typeof(CasesConverter<Message>))]
[JsonCase(typeof(MessageBuild), "build")]
[JsonCase(typeof(MessageReply), "reply")]
[JsonCase(typeof(MessageReplyFlag), "replyFlag")]
[JsonCase(typeof(MessageEvent), "event")]
[JsonCase(typeof(MessageOutput), "output")]
public abstract record Message
{
    private protected Message() { }
}

/// <summary>Answers <see cref="RequestBuild"/> with the Helper's build.</summary>
public sealed record MessageBuild(ulong Id, string Build) : Message;

/// <summary>Answers a request with its JSON payload; null is the empty (success) reply.</summary>
public sealed record MessageReply(ulong Id, string? Json = null) : Message;

/// <summary>Answers <see cref="RequestExitWhenIdle"/>.</summary>
public sealed record MessageReplyFlag(ulong Id, bool Value) : Message;

/// <summary>An event of a run this connection started.</summary>
public sealed record MessageEvent(string Json) : Message;

/// <summary>A batch of output lines of a run this connection started.</summary>
public sealed record MessageOutput(string Json) : Message;
