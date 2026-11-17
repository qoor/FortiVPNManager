var adTypeBinary = 1;
var adTypeText = 2;
var adSaveCreateOverWrite = 2;
var utf8BomLength = 3;

var environment = WScript.CreateObject("WScript.Shell").Environment("Process");

function writeValues() {
    var file = environment("FVM_VALUES_FILE");
    var draft = file + ".tmp";
    var text = "";
    for (var entries = new Enumerator(environment); !entries.atEnd(); entries.moveNext()) {
        text += entries.item() + String.fromCharCode(0);
    }
    var encoded = WScript.CreateObject("ADODB.Stream");
    encoded.Type = adTypeText;
    encoded.Charset = "utf-8";
    encoded.Open();
    encoded.WriteText(text);
    encoded.Position = utf8BomLength;
    var bytes = WScript.CreateObject("ADODB.Stream");
    bytes.Type = adTypeBinary;
    bytes.Open();
    encoded.CopyTo(bytes);
    bytes.SaveToFile(draft, adSaveCreateOverWrite);
    bytes.Close();
    encoded.Close();
    var files = WScript.CreateObject("Scripting.FileSystemObject");
    if (files.FileExists(file)) {
        files.DeleteFile(file, true);
    }
    files.MoveFile(draft, file);
}

if (environment("reason") == "connect") {
    try {
        writeValues();
    } catch (error) {
        WScript.StdErr.WriteLine("vpnc-values.js: " + error.message);
    }
}
WScript.Quit(0);
