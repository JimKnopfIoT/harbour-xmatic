#pragma once

#include <QString>

// Fetches — or creates on first run — the 32-byte store key from Sailfish
// Secrets and returns it base64-encoded, ready for the core's config. Returns
// an empty string when the secrets service is unavailable (no daemon, denied
// permission): the core then runs with unencrypted stores — degrade, not
// block. Blocking, called once at startup before the core exists.
QString obtainStoreKey();
