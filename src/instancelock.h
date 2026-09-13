#ifndef INSTANCELOCK_H
#define INSTANCELOCK_H

#include <QString>

/// Takes the single-instance lock, or reports that another process holds it.
/// Held for the life of the process and released by the kernel, however it dies.
bool acquireInstanceLock(const QString &dataDirectory);

/// Asks the running instance to come to the front, over the share dialog's own
/// name. Best effort: otherwise the user taps the icon again.
void raiseRunningInstance();

/// Hands a link to the application and reports whether it arrived. Where none
/// is running, the call itself starts one through the D-Bus service file, which
/// runs it under the app's own profile - the point of the whole exercise, since
/// the process delivering the link has an identity of its own and none of the
/// app's rights. Blocking and patient: a cold start has to fit inside it.
bool deliverLink(const QString &link);

#endif // INSTANCELOCK_H
