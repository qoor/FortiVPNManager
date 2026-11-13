using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

/// <summary>
/// Messages over the Windows Helper service's named pipe (components/contract/src/helper_pipe.rs),
/// in XPC's place on macOS, framed by <see cref="MessageFraming"/>. The app numbers its requests;
/// the Helper answers each one once with that number, and sends a run's events and output in the
/// order they happened on the same pipe. The JSON payloads are passed to and from the runtime
/// unread.
/// </summary>
public static class HelperPipe
{
    /// <summary>The pipe's name as a client opens it on this computer. The system account owns it.</summary>
    public const string Name = "com.qoor.FortiVPNManager.Helper";

    /// <summary>Sent by the app. <c>Id</c> names the request in the Helper's answer.</summary>
    [Cases]
    public abstract record Request
    {
        private Request() { }

        /// <summary>
        /// The build check, answered by <see cref="Message.Build"/>; <c>App</c> is the app's build.
        /// Goes between builds, so its shape and its answer's never change.
        /// </summary>
        public sealed record Build(ulong Id, string App) : Request;

        /// <summary><c>Json</c> is a run request. Answered by <see cref="Message.Reply"/>.</summary>
        public sealed record Connect(ulong Id, string Json) : Request
        {
            public override string ToString() => $"Connect {{ Id = {Id}, Json = <redacted> }}";
        }

        public sealed record SubmitOtp(ulong Id, string Attempt, string Code) : Request
        {
            public override string ToString() => $"SubmitOtp {{ Id = {Id}, Attempt = {Attempt}, Code = <redacted> }}";
        }

        public sealed record Disconnect(ulong Id, string Attempt, string Reason) : Request;
        public sealed record Status(ulong Id) : Request;

        /// <summary>
        /// Answered by <see cref="Message.ReplyFlag"/>; the Windows Helper always answers false. Goes
        /// between builds, so its shape and its answer's never change.
        /// </summary>
        public sealed record ExitWhenIdle(ulong Id) : Request;

        public sealed record ReadBypassRoutes(ulong Id) : Request;
        public sealed record WriteBypassRoutes(ulong Id, string Json) : Request;
    }

    /// <summary>Sent by the Helper.</summary>
    [Cases]
    public abstract record Message
    {
        private Message() { }

        /// <summary>Answers <see cref="Request.Build"/> with the Helper's build.</summary>
        public sealed record Build(ulong Id, [property: JsonPropertyName("build")] string HelperBuild) : Message;

        /// <summary>Answers a request with its JSON payload; null is the empty (success) reply.</summary>
        public sealed record Reply(ulong Id, string? Json = null) : Message;

        /// <summary>Answers <see cref="Request.ExitWhenIdle"/>.</summary>
        public sealed record ReplyFlag(ulong Id, bool Value) : Message;

        /// <summary>An event of a run this connection started.</summary>
        public sealed record Event(string Json) : Message;

        /// <summary>A batch of output lines of a run this connection started.</summary>
        public sealed record Output(string Json) : Message;
    }
}
