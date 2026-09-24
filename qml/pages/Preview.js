// The last event of a room in one line, for the chat list and the notification
// banner alike. Not a library: qsTr needs a translation context, and this file
// is it.

function line(kind, text) {
    switch (kind) {
    case "text": return text
    case "emote": return "* " + text
    case "image": return qsTr("Picture")
    case "video": return qsTr("Video")
    case "audio": return qsTr("Voice message")
    case "file": return qsTr("File")
    case "location": return qsTr("Location")
    case "poll": return text.length > 0 ? qsTr("Poll: %1").arg(text) : qsTr("Poll")
    case "encrypted": return qsTr("Encrypted message")
    default: return ""
    }
}
