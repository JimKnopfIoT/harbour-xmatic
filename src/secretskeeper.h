#pragma once

#include <QString>

// Fetches — or creates on first run — the 32-byte store key from Sailfish
// Secrets and returns it base64-encoded, ready for the core's config. Returns
// an empty string when the key cannot be had: the core then runs with
// unencrypted stores where nothing is encrypted yet — degrade, not block —
// and reports an encrypted session as "locked" (retry, never a re-login).
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
QString obtainStoreKey(const QString &dataDirectory);

// True if the data directory holds anything written under a store key: the
// `.encrypted` marker of the SQLite stores, or a session file in its
// encrypted envelope.
bool encryptedDataPresent(const QString &dataDirectory);
