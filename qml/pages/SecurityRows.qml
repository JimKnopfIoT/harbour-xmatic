import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// The four lines that make up "how safe is this device", in one place.
//
// Used by the encryption page and by the page that interrupts the start. Two
// copies would drift, and the two are read minutes apart by the same person —
// if they ever disagreed, neither would be believed again.
Column {
    id: rows

    width: parent.width
    spacing: Theme.paddingMedium

    SecurityRow {
        label: qsTr("Backup")
        level: SecurityStatus.backupLevel(matrix)
        detail: {
            var s = matrix.encryptionStatus
            if (s.backupEnabled) {
                return qsTr("Your room keys are backed up.")
            }
            // Null, not false: the core could not reach the server. Saying
            // "there is no backup" there is a claim about the account made
            // from a failed request.
            if (s.backupOnServer === undefined || s.backupOnServer === null) {
                return qsTr("The server could not be asked whether a backup exists.")
            }
            return s.backupOnServer
                    ? qsTr("The backup is on the server but not unlocked on this device.")
                    : qsTr("There is no key backup. Without one, messages become unreadable when this device is gone.")
        }
    }

    SecurityRow {
        label: qsTr("Recovery")
        level: SecurityStatus.recoveryLevel(matrix)
        detail: {
            if (matrix.encryptionStatus.recovery === "enabled") {
                return qsTr("Recovery is set up.")
            }
            // The key was accepted and the state has not caught up yet. Without
            // this the line looks untouched and people enter the key again -
            // which is what a tester did, and the app taught them to.
            if (matrix.recoverySettling) {
                return qsTr("Recovery key accepted — finishing. This can take a moment.")
            }
            // "Not set up" and "not unlocked here" are different situations and
            // want opposite advice. Telling somebody with no recovery at all to
            // enter their recovery key sends them looking for something that
            // does not exist - and the line above says red, which is the honest
            // colour for it.
            if (matrix.encryptionStatus.recovery === "disabled") {
                return qsTr("There is no recovery for this account yet. Set up a key backup to create one.")
            }
            return qsTr("Enter your recovery key to unlock the backup on this device.")
        }
    }

    SecurityRow {
        label: qsTr("Cross-signing")
        level: SecurityStatus.crossSigningLevel(matrix)
        detail: matrix.encryptionStatus.crossSigned
                ? qsTr("This device is signed as yours.")
                : qsTr("Others see this device as unverified. The recovery key or a verification from another device settles it.")
    }

    SecurityRow {
        label: qsTr("Local storage")
        level: SecurityStatus.storageLevel(matrix)
        detail: {
            var s = matrix.storageStatus
            if (s.encrypted) {
                return qsTr("Session and message database are encrypted on this device.")
            }
            if (s.keyAvailable) {
                return qsTr("They lie unencrypted because they were created before this app could encrypt them. An existing database cannot be encrypted in place.")
            }
            return matrix.secretsDaemonPresent
                    ? qsTr("They lie unencrypted because the system's secure storage did not hand out a key.")
                    : qsTr("They lie unencrypted because this system is missing the service that keeps encryption keys.")
        }
    }
}
