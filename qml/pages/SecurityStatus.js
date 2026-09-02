.pragma library

// One reading of "how safe is this device", shared by every page that shows
// it: the pages must never disagree, which is how the pinned banner broke.

// Green: in order. Orange: a fault the user can clear. Red: missing, and not
// reachable without acting.
var GREEN = "green"
var ORANGE = "orange"
var RED = "red"
// Nothing has been answered yet. Not a level — a caller must not paint it and
// must not interrupt anybody over it.
var UNKNOWN = "unknown"

// Whether the state has arrived at all: right after the start the map is empty,
// and "unknown" for half a second must not put a page in front of the user.
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
    // Null is not "no": a failed request used to read as "there is no key backup",
    // the red line that interrupts the start.
    if (s.backupOnServer === undefined || s.backupOnServer === null) {
        return UNKNOWN
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
    // "incomplete" and anything unforeseen land here. An answer nobody recognises
    // may not be painted green.
    default: return ORANGE
    }
}

function crossSigningLevel(matrix) {
    var s = matrix.encryptionStatus
    if (!s || s.recovery === undefined) {
        return UNKNOWN
    }
    // Orange, not red: the core answers yes/no, so "not signed into an identity"
    // cannot be told from "there is no identity", and the milder reading is honest.
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
    // A key at hand and older data is one sign-out away - the user's decision.
    // Without a key the system is not delivering what it takes.
    return s.keyAvailable ? ORANGE : RED
}

// The worst of the four: showing the better half would claim a protection that
// covers half of it.
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

// `theme` and `lightOnDark` are handed in because a `.pragma library` can
// import neither. Silica has no green or orange, so those are fixed per scheme.
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
