#ifndef INSTANCELOCK_H
#define INSTANCELOCK_H

#include <QString>

/// Takes the single-instance lock, or reports that another process holds it.
/// Held for the life of the process and released by the kernel, however it dies.
bool acquireInstanceLock(const QString &dataDirectory);

/// Asks the running instance to come to the front, over the share dialog's own
/// name. Best effort: otherwise the user taps the icon again.
void raiseRunningInstance();

#endif // INSTANCELOCK_H
