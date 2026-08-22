#ifndef INSTANCELOCK_H
#define INSTANCELOCK_H

#include <QString>

/// Takes the single-instance lock for `dataDirectory`, or reports that another
/// process already holds it.
///
/// Returns true when this process may proceed. False means a live instance is
/// already running on the same store; the caller has to leave without touching
/// it. The lock is held for the rest of the process's life and released by the
/// kernel when it dies, however it dies.
bool acquireInstanceLock(const QString &dataDirectory);

/// Asks the running instance to come to the front, over the same D-Bus name
/// the share dialog uses. Best effort: if it does not answer, the user simply
/// taps the icon again.
void raiseRunningInstance();

#endif // INSTANCELOCK_H
