#ifndef PUSHWAKE_H
#define PUSHWAKE_H

/// The environment variable the push activation sets. Not an argument: SailJail
/// matches the command line against the desktop templates exactly.
#define XMATIC_PUSH_WAKE_ENV "XMATIC_PUSH_WAKE"

/// Left in the data directory by an app that finds the store taken by a wake-up;
/// the wake-up quits, the app removes it.
#define XMATIC_WAKE_YIELD_FILE "wake-yield"

/// Runs the process as a push connector - no window, no QML. The core claims
/// the name, the banners go up here, and the process exits.
int runPushWake(int argc, char *argv[]);

/// Sets FLATPAK_ID for Leghorn's bus library on Sailfish OS before 5.1.
/// Before Qt and any other thread.
void pushPrelude(const char *argv0);

#include <QString>

#include <QHash>

/// Closes the banners woken processes raised.
void closePushBanners(const QString &dataDirectory);

/// Banner action key to room id, from the banners still recorded.
QHash<QString, QString> pushBannerRooms(const QString &dataDirectory);

void publishPushNotice(const QString &dataDirectory);

#endif // PUSHWAKE_H
