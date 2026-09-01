#ifndef PUSHWAKE_H
#define PUSHWAKE_H

/// The environment variable the D-Bus activation for a push sets.
///
/// An environment variable and not an argument, because SailJail matches the
/// command line it is handed against the desktop file's `Exec` and `ExecDBus`
/// templates exactly — one extra word is refused with "Command line does not
/// match templates". It does not inspect the environment, so the mode travels
/// there and both templates stay identical and unflagged.
#define XMATIC_PUSH_WAKE_ENV "XMATIC_PUSH_WAKE"

/// Runs the process as a push connector rather than as the app: no window, no
/// QML, no Qt Quick. Returns the process's exit code.
///
/// Started by D-Bus when a push arrives for
/// `org.unifiedpush.Connector.xmatic` and this app is not running. It claims
/// that name, waits a bounded time for the message, turns it into a
/// notification and exits — a connector that stays resident only costs
/// battery, and the distributor activates it again for the next one.
int runPushWake(int argc, char *argv[]);

#endif // PUSHWAKE_H
