using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The app's secrets as the user's generic credentials in Credential Manager, kept on this PC only.
/// Each item is named after its macOS Keychain service and account, so the names read the same on
/// both systems.
/// </summary>
internal static class SecretStore
{
    private const uint GenericType = 1;
    private const uint PersistLocalMachine = 2;
    private const int NotFound = 1168;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct Credential
    {
        public uint Flags;
        public uint Type;
        public string TargetName;
        public string? Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public string? TargetAlias;
        public string? UserName;
    }

    [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, uint type, uint flags, out IntPtr credential);

    [DllImport("advapi32.dll", EntryPoint = "CredWriteW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredWrite(ref Credential credential, uint flags);

    [DllImport("advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredDelete(string target, uint type, uint flags);

    [DllImport("advapi32.dll")]
    private static extern void CredFree(IntPtr buffer);

    public static PortResponse Read(SecretItem item)
    {
        if (!CredRead(TargetName(item), GenericType, 0, out var found))
        {
            var error = Marshal.GetLastWin32Error();
            return error == NotFound ? new PortResponse.Secret() : Failed(error);
        }
        try
        {
            var credential = Marshal.PtrToStructure<Credential>(found);
            var bytes = new byte[credential.CredentialBlobSize];
            // An empty secret comes without a blob.
            if (bytes.Length > 0)
            {
                Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            }
            var value = Encoding.UTF8.GetString(bytes);
            Array.Clear(bytes);
            return new PortResponse.Secret(value);
        }
        finally
        {
            CredFree(found);
        }
    }

    /// <summary>Reading a credential never asks the user, so this reads it and drops the value.</summary>
    public static PortResponse Exists(SecretItem item)
    {
        if (CredRead(TargetName(item), GenericType, 0, out var found))
        {
            CredFree(found);
            return new PortResponse.HasSecret(true);
        }
        var error = Marshal.GetLastWin32Error();
        return error == NotFound ? new PortResponse.HasSecret(false) : Failed(error);
    }

    public static PortResponse Write(string value, SecretItem item)
    {
        var bytes = Encoding.UTF8.GetBytes(value);
        var blob = Marshal.AllocHGlobal(Math.Max(bytes.Length, 1));
        try
        {
            Marshal.Copy(bytes, 0, blob, bytes.Length);
            var credential = new Credential
            {
                Type = GenericType,
                TargetName = TargetName(item),
                CredentialBlobSize = (uint)bytes.Length,
                CredentialBlob = blob,
                Persist = PersistLocalMachine,
                UserName = Account(item),
            };
            return CredWrite(ref credential, 0)
                ? new PortResponse.SecretChanged()
                : Failed(Marshal.GetLastWin32Error());
        }
        finally
        {
            Marshal.Copy(new byte[bytes.Length], 0, blob, bytes.Length);
            Marshal.FreeHGlobal(blob);
            Array.Clear(bytes);
        }
    }

    /// <summary>Deleting a missing item succeeds.</summary>
    public static PortResponse Delete(SecretItem item)
    {
        if (CredDelete(TargetName(item), GenericType, 0))
        {
            return new PortResponse.SecretChanged();
        }
        var error = Marshal.GetLastWin32Error();
        return error == NotFound ? new PortResponse.SecretChanged() : Failed(error);
    }

    /// <summary>The credential's name: the macOS Keychain service and account of the item.</summary>
    public static string TargetName(SecretItem item) => $"{Service(item)}/{Account(item)}";

    /// <summary>The macOS Keychain service of this secret. Changing it loses the items already saved.</summary>
    public static string Service(SecretItem item) => item switch
    {
        SecretItem.VpnPassword => "com.qoor.FortiVPNManager.vpn",
        SecretItem.GmailAppPassword => "com.qoor.FortiVPNManager.gmail",
        SecretItem.GoogleRefreshToken or SecretItem.GoogleClientSecret => "com.qoor.FortiVPNManager.google",
        _ => throw new ArgumentOutOfRangeException(nameof(item)),
    };

    /// <summary>The macOS Keychain account of this secret, kept like <see cref="Service"/>.</summary>
    public static string Account(SecretItem item) => item switch
    {
        SecretItem.VpnPassword vpn => $"{vpn.Username}@{vpn.Host}",
        SecretItem.GmailAppPassword gmail => gmail.Address,
        SecretItem.GoogleRefreshToken token => $"refresh-token:{token.Address.ToLowerInvariant()}",
        SecretItem.GoogleClientSecret client => $"client-secret:{client.ClientId}",
        _ => throw new ArgumentOutOfRangeException(nameof(item)),
    };

    /// <summary>As Windows describes the error, in its language.</summary>
    private static PortResponse Failed(int error) =>
        new PortResponse.SecretStoreFailed(new SecretStoreFailure(new Win32Exception(error).Message));
}
