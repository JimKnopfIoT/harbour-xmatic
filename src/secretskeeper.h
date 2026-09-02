#pragma once

#include <QString>

/// How the attempt at the store key ended. One empty string for four
/// situations is why a factory-fresh device silently got a plaintext store.
enum class StoreKeyState {
    /// The key is in hand.
    Available,
    /// Encrypted data exists on disk and the key could not be read. Never mint
    /// a new one here: the session is locked, and the user retries.
    Locked,
    /// The secrets daemon is not installed on this system. No amount of
    /// retrying helps; the user has to install a package.
    NoDaemon,
    /// The daemon is there and still would not deliver a key — a locked
    /// collection, a dismissed dialog, a missing plugin. Retriable.
    Unavailable,
};

/// The outcome of `obtainStoreKey`, with the daemon's own words for the reason.
struct StoreKeyResult {
    StoreKeyState state = StoreKeyState::Unavailable;
    /// Base64, ready for the core's config. Only set for `Available`, and the
    /// caller wipes it once the core has taken its copy.
    QString key;
    /// secretsd's error code and message, carrying no secret material. Empty
    /// where nothing failed.
    int errorCode = 0;
    QString errorMessage;
};

// Fetches - or on first run creates - the 32-byte key. System interaction is
// allowed so secretsd can run its device-lock dialog; blocking, called at start.
StoreKeyResult obtainStoreKey(const QString &dataDirectory);

// True if the directory holds anything written under a key: the `.encrypted`
// marker, or a session file in its envelope.
bool encryptedDataPresent(const QString &dataDirectory);

// True if there is a session or a store at all. The gate asks "would a new
// store be created", and an install predating the key must keep working.
bool localDataPresent(const QString &dataDirectory);

// True if this system has the secrets daemon. Asked of the session bus:
// inside Sailjail the app sees neither binary nor service file.
bool secretsDaemonPresent();
