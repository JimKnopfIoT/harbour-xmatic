#ifndef PUSHWAKE_H
#define PUSHWAKE_H

/// The environment variable the push activation sets. Not an argument: SailJail
/// matches the command line against the desktop templates exactly.
#define XMATIC_PUSH_WAKE_ENV "XMATIC_PUSH_WAKE"

/// Left in the data directory by an app that finds the store taken by a wake-up;
/// the wake-up removes it and quits.
#define XMATIC_WAKE_YIELD_FILE "wake-yield"

/// Runs the process as a push connector - no window, no QML. Claims the name,
/// waits a bounded time, raises the notification and exits.
int runPushWake(int argc, char *argv[]);

#endif // PUSHWAKE_H
