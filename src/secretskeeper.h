#pragma once

#include <QString>

/// How the attempt to get the store key ended.
///
/// Until 0.25.2 this was one empty `QString` for four different situations,
/// and `main()` could not tell them apart — so a factory-fresh device whose
/// image simply does not ship the secrets daemon got a plaintext store, with
/// nothing but a journal line to say so. What the caller has to distinguish is
/// "this system cannot do it" from "not right now".
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

// Fetches — or creates on first run — the 32-byte store key from Sailfish
// Secrets and reports how that went.
//
// The collection is bound to the device lock, and secretsd only unlocks it
// through its own authentication — a system dialog the first time after a
// boot. The requests therefore allow system interaction; without it the
// read fails with "collection is locked" at every start after a reboot,
// which is what happened in 0.16–0.18. Blocking; called at startup before
// the core exists and again on the user's retry.
//
// `dataDirectory` is the app's data location: whether encrypted data already
// exists there decides if a missing key may be minted. It may only when
// nothing on disk was written under a key — otherwise a key that is merely
// unreachable right now would be replaced, and every store under the old one
// lost for good.
StoreKeyResult obtainStoreKey(const QString &dataDirectory);

// True if the data directory holds anything written under a store key: the
// `.encrypted` marker of the SQLite stores, or a session file in its
// encrypted envelope.
bool encryptedDataPresent(const QString &dataDirectory);

// True if the data directory holds a session or a store at all, encrypted or
// not.
//
// The question the gate asks is not "is anything encrypted" but "would a new
// store be created right now". An install that predates the store key has
// plaintext data and no key, and must keep working — blocking it would strand
// the very users the migration page is meant to guide. Only a directory with
// nothing in it is a fresh start whose store may still be refused.
bool localDataPresent(const QString &dataDirectory);

// True if this system has the Sailfish Secrets daemon at all.
//
// Asked of the filesystem rather than of a failed request, because the two
// answer different questions: a request fails just as well when the collection
// is locked, and the difference decides between "install this package" and
// "try again". Measured on hardware: the Sailfish 5.2 image ships only the
// client library, while 5.1 and 4.6 carry the daemon.
bool secretsDaemonPresent();
