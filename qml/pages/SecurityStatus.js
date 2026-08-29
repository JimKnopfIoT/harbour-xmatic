.pragma library

// One reading of "how safe is this device", shared by every page that shows it.
//
// The point of putting it here is that the encryption page, the lamps in the
// headers and the page that interrupts the start must never disagree. Two
// copies of this logic is how the pinned banner broke: one path asked the
// server, the other only the local state, and they drifted apart unnoticed.

// Green: in order. Orange: a fault the user can clear. Red: missing, and not
// reachable without acting.
var GREEN = "green"
var ORANGE = "orange"
var RED = "red"
// Nothing has been answered yet. Not a level — a caller must not paint it and
// must not interrupt anybody over it.
var UNKNOWN = "unknown"

// Whether the encryption state has arrived at all. Right after the start the
// map is empty, and "backup unknown" for half a second must not put a page in
// front of the user.
function known(matrix) {
    return matrix.encryptionStatus
            && matrix.encryptionStatus.recovery !== undefined
            && matrix.storageStatus
            && matrix.storageStatus.encrypted !== undefined
}

function backupLevel(matrix) {
    var s = matrix.encryptionStatus
    if (!s || s.recovery === undefined) {
        return UNKNOWN
    }
    if (s.backupEnabled) {
        return GREEN
    }
    // On the server but not unlocked here: the keys exist, this device just
    // cannot reach them. That is a fault, not an absence.
    return s.backupOnServer ? ORANGE : RED
}

function recoveryLevel(matrix) {
    var s = matrix.encryptionStatus
    if (!s || s.recovery === undefined) {
        return UNKNOWN
    }
    switch (s.recovery) {
    case "enabled": return GREEN
    case "disabled": return RED
    // "incomplete" and anything unforeseen land here. An answer nobody
    // recognises may not be painted green: a statement without knowledge must
    // not reassure.
    default: return ORANGE
    }
}

function crossSigningLevel(matrix) {
    var s = matrix.encryptionStatus
    if (!s || s.recovery === undefined) {
        return UNKNOWN
    }
    // Orange rather than red, and deliberately so: the core answers a plain
    // yes/no, so "an identity exists and this device is not signed into it"
    // cannot be told apart from "there is no identity at all". The first is a
    // fault, the second an absence, and without the distinction the milder
    // reading is the honest one.
    return s.crossSigned ? GREEN : ORANGE
}

function storageLevel(matrix) {
    var s = matrix.storageStatus
    if (!s || s.encrypted === undefined) {
        return UNKNOWN
    }
    if (s.encrypted) {
        return GREEN
    }
    // A key is at hand and the data is merely older than it: one sign-out
    // fixes that, so it is the user's decision, not the system's failure.
    // Without a key the system is not delivering what it takes.
    return s.keyAvailable ? ORANGE : RED
}

// The worst of the four. The same rule the storage line has followed since
// 0.21.0: showing the better half would claim a protection that covers half of
// it.
function overall(matrix) {
    var levels = [backupLevel(matrix), recoveryLevel(matrix),
                  crossSigningLevel(matrix), storageLevel(matrix)]
    var worst = GREEN
    for (var i = 0; i < levels.length; ++i) {
        if (levels[i] === UNKNOWN) {
            return UNKNOWN
        }
        if (levels[i] === RED) {
            return RED
        }
        if (levels[i] === ORANGE) {
            worst = ORANGE
        }
    }
    return worst
}

// Whether there is anything to lead the user to. Unknown is not "not green":
// an unanswered state interrupts nobody.
function needsAttention(matrix) {
    var level = overall(matrix)
    return level === ORANGE || level === RED
}

// `theme` is Silica's Theme and `lightOnDark` its colour scheme, both handed in
// because a `.pragma library` can import neither — and the enum constant that
// names the scheme lives on the type, not on the object, so the caller resolves
// it: `SecurityStatus.color(level, Theme, Theme.colorScheme === Theme.LightOnDark)`.
//
// Red comes from the ambience itself. Silica has no green and no orange, so
// those are fixed values, one pair per scheme — a light ambience needs the
// darker tone to stay legible on a bright background.
function color(level, theme, lightOnDark) {
    switch (level) {
    case GREEN:
        return lightOnDark ? "#4cd964" : "#1b7f3b"
    case ORANGE:
        return lightOnDark ? "#ff9f0a" : "#a85c00"
    case RED:
        return theme.errorColor
    default:
        return theme.secondaryColor
    }
}
