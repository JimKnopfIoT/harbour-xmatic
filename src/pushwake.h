#ifndef PUSHWAKE_H
#define PUSHWAKE_H

#include <QProcessEnvironment>

/// The environment variable the push activation sets. Not an argument: SailJail
/// matches the command line against the desktop templates exactly.
#define XMATIC_PUSH_WAKE_ENV "XMATIC_PUSH_WAKE"

/// Left in the data directory by an app that finds the store taken by a wake-up;
/// the wake-up quits, the app removes it.
#define XMATIC_WAKE_YIELD_FILE "wake-yield"

/// How long a wake-up asked to yield may take: the push in hand, then the tokens
/// saved (`YIELD_FINISH` + `YIELD_SAVE` in the core). The app waits half a second more.
#define XMATIC_WAKE_YIELD_MS 10000

/// Runs the process as a push connector - no window, no QML. The core claims
/// the name, the banners go up here, and the process exits.
int runPushWake(int argc, char *argv[]);

/// Sets FLATPAK_ID for Leghorn's bus library on Sailfish OS before 5.1.
/// Before Qt and any other thread; never in a helper process.
void pushPrelude(const char *argv0);

/// The environment for a helper process, without what `pushPrelude` set for
/// this one.
QProcessEnvironment helperEnvironment();

#endif // PUSHWAKE_H
