// The store key in a Sailfish Secrets collection: 32 random bytes, device-lock
// bound, owner-only. A local key for the stores, not an account credential.

#include "secretskeeper.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDir>
#include <QFile>

#include <Sailfish/Secrets/createcollectionrequest.h>
#include <Sailfish/Secrets/result.h>
#include <Sailfish/Secrets/secret.h>
#include <Sailfish/Secrets/secretmanager.h>
#include <Sailfish/Secrets/storedsecretrequest.h>
#include <Sailfish/Secrets/storesecretrequest.h>

using namespace Sailfish::Secrets;

namespace {

const QString collectionName = QStringLiteral("xmatic");
const QString secretName = QStringLiteral("storeKey");

Secret::Identifier keyIdentifier()
{
    return Secret::Identifier(secretName,
                              collectionName,
                              SecretManager::DefaultEncryptedStoragePluginName);
}

/// Overwrites a buffer so the write survives the optimiser. Only on an unshared
/// one: `data()` detaches and would wipe a fresh copy instead.
void wipe(QByteArray &bytes)
{
    if (bytes.isEmpty()) {
        return;
    }
    volatile char *raw = bytes.data();
    for (int i = 0; i < bytes.size(); ++i) {
        raw[i] = '\0';
    }
}

QByteArray randomKey()
{
    // /dev/urandom rather than qrand: Qt 5.6 has no QRandomGenerator, and a
    // store key must never come from a seedable PRNG.
    QFile urandom(QStringLiteral("/dev/urandom"));
    if (!urandom.open(QIODevice::ReadOnly)) {
        return QByteArray();
    }
    QByteArray key = urandom.read(32);
    return key.size() == 32 ? key : QByteArray();
}

} // namespace

bool encryptedDataPresent(const QString &dataDirectory)
{
    if (QFile::exists(dataDirectory + QStringLiteral("/store/.encrypted"))) {
        return true;
    }
    // The encrypted lists can be the only thing on disk - allowed a caller, then
    // signed out. Minting a fresh key over them makes them unreadable for good.
    if (QFile::exists(dataDirectory + QStringLiteral("/private.json"))) {
        return true;
    }
    QFile session(dataDirectory + QStringLiteral("/session.json"));
    if (!session.exists()) {
        return false;
    }
    if (!session.open(QIODevice::ReadOnly)) {
        // There and unreadable is not "nothing is encrypted": this answer decides
        // whether a new key goes over the old one, so it fails towards doing nothing.
        return true;
    }
    // The envelope starts with the core's one top-level key, a plaintext session
    // with the homeserver. Only the shape is looked at.
    QByteArray head = session.read(64);
    const bool envelope = head.contains("\"encrypted\"");
    wipe(head);
    return envelope;
}

bool localDataPresent(const QString &dataDirectory)
{
    if (QFile::exists(dataDirectory + QStringLiteral("/session.json"))
        || QFile::exists(dataDirectory + QStringLiteral("/private.json"))) {
        return true;
    }
    QDir store(dataDirectory + QStringLiteral("/store"));
    if (!store.exists()) {
        return false;
    }
    // The core's `prepare()` creates this directory, so "exists and empty" is a
    // fresh install. Hidden entries count - `.encrypted` is one.
    return !store.entryList(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden)
                .isEmpty();
}

bool secretsDaemonPresent()
{
    // Asked of the bus, not the filesystem: inside Sailjail the app sees neither
    // the binary nor the service file even where both are installed.
    const QDBusConnection bus = QDBusConnection::sessionBus();
    if (bus.isConnected()) {
        if (QDBusConnectionInterface *iface = bus.interface()) {
            const QString name =
                QStringLiteral("org.sailfishos.secrets.daemon.discovery");
            if (iface->isServiceRegistered(name).value()) {
                return true;
            }
            // `ListActivatableNames` by hand - Qt 5.6 has no accessor. Bounded: this runs
            // before the window is up and a silent bus must not hold the app.
            QDBusMessage call = QDBusMessage::createMethodCall(
                QStringLiteral("org.freedesktop.DBus"),
                QStringLiteral("/org/freedesktop/DBus"),
                QStringLiteral("org.freedesktop.DBus"),
                QStringLiteral("ListActivatableNames"));
            const QDBusMessage reply = bus.call(call, QDBus::Block, 2000);
            if (reply.type() == QDBusMessage::ReplyMessage) {
                return reply.arguments().value(0).toStringList().contains(name);
            }
            // The bus is there but would not answer. Fall through to the files
            // rather than claim anything.
        }
    }
    // No bus to ask, so fall back to the files. The doubtful answer is "it is
    // there": sending somebody to install what they have is the worse mistake.
    return QFile::exists(QStringLiteral("/usr/bin/sailfishsecretsd"))
           || QFile::exists(QStringLiteral(
                  "/usr/share/dbus-1/services/"
                  "org.sailfishos.secrets.daemon.discovery.service"));
}

namespace {

/// Fills in the reason from a request result, for the journal and the UI.
StoreKeyResult failure(StoreKeyState state, const Result &result)
{
    StoreKeyResult outcome;
    outcome.state = state;
    outcome.errorCode = static_cast<int>(result.errorCode());
    // secretsd's own text. It names collections and plugins, never key
    // material — the same string this app has logged since 0.18.1.
    outcome.errorMessage = result.errorMessage();
    return outcome;
}

} // namespace

StoreKeyResult obtainStoreKey(const QString &dataDirectory)
{
    SecretManager manager;

    // The common case first. System interaction is allowed so secretsd can run its
    // device-lock dialog once per boot; without it a locked collection just fails.
    {
        StoredSecretRequest read;
        read.setManager(&manager);
        read.setIdentifier(keyIdentifier());
        read.setUserInteractionMode(SecretManager::SystemInteraction);
        read.startRequest();
        read.waitForFinished();
        const Result result = read.result();
        if (result.code() == Result::Succeeded) {
            // Detached before anything else: `secret().data()` shares with the
            // request's own buffer, and wiping a shared one wipes a copy.
            QByteArray data = read.secret().data();
            data.detach();
            if (data.size() == 32) {
                // The base64 is wiped too: `toBase64()` builds a second buffer holding the
                // same key in another alphabet.
                QByteArray encoded64 = data.toBase64();
                const QString encoded = QString::fromLatin1(encoded64);
                wipe(encoded64);
                wipe(data);
                qInfo("xmatic: store key loaded from the device's secrets storage");
                StoreKeyResult outcome;
                outcome.state = StoreKeyState::Available;
                outcome.key = encoded;
                return outcome;
            }
            // A key of the wrong size cannot have encrypted anything, so replacing it
            // loses nothing.
            qWarning("xmatic: stored key has the wrong size, creating a new one");
        } else if (encryptedDataPresent(dataDirectory)) {
            // Something on disk was written under a key and this read did not deliver it.
            // Minting now would replace the key that data needs; the core answers `locked`.
            qWarning("xmatic: store key not available (%d: %s); encrypted data waits for it",
                     static_cast<int>(result.errorCode()),
                     qPrintable(result.errorMessage()));
            // Locked even where the daemon is missing: this state is what forbids minting
            // a key, and that rule may not depend on why the read failed.
            return failure(StoreKeyState::Locked, result);
        } else {
            qInfo("xmatic: no store key yet (%d: %s), creating one",
                  static_cast<int>(result.errorCode()),
                  qPrintable(result.errorMessage()));
        }
    }

    // First run, or the collection is gone: create it, then the key. Creation
    // failing because it exists is fine - the store below decides.
    {
        CreateCollectionRequest create;
        create.setManager(&manager);
        create.setCollectionName(collectionName);
        create.setCollectionLockType(CreateCollectionRequest::DeviceLock);
        create.setDeviceLockUnlockSemantic(SecretManager::DeviceLockKeepUnlocked);
        create.setAccessControlMode(SecretManager::OwnerOnlyMode);
        create.setStoragePluginName(SecretManager::DefaultEncryptedStoragePluginName);
        create.setEncryptionPluginName(SecretManager::DefaultEncryptedStoragePluginName);
        create.setUserInteractionMode(SecretManager::SystemInteraction);
        create.startRequest();
        create.waitForFinished();
        if (create.result().code() != Result::Succeeded
            && create.result().errorCode() != Result::CollectionAlreadyExistsError) {
            qWarning("xmatic: secrets collection could not be created (%d: %s)",
                     static_cast<int>(create.result().errorCode()),
                     qPrintable(create.result().errorMessage()));
        }
    }

    QByteArray key = randomKey();
    if (key.isEmpty()) {
        qWarning("xmatic: no randomness source; stores stay unencrypted");
        StoreKeyResult outcome;
        outcome.state = StoreKeyState::Unavailable;
        outcome.errorMessage = QStringLiteral("no randomness source");
        return outcome;
    }

    Secret secret(keyIdentifier());
    // The secret takes its own copy; ours is wiped below, the request's is out
    // of reach - the same residual the password login documents.
    secret.setData(key);
    key.detach();

    StoreSecretRequest store;
    store.setManager(&manager);
    store.setSecretStorageType(StoreSecretRequest::CollectionSecret);
    store.setSecret(secret);
    store.setUserInteractionMode(SecretManager::SystemInteraction);
    store.startRequest();
    store.waitForFinished();

    if (store.result().code() != Result::Succeeded) {
        // The error string comes from secretsd and carries no secret material.
        const bool daemon = secretsDaemonPresent();
        qWarning("xmatic: secrets storage unavailable (%d: %s); daemon installed: %d",
                 static_cast<int>(store.result().errorCode()),
                 qPrintable(store.result().errorMessage()),
                 daemon ? 1 : 0);
        wipe(key);
        return failure(daemon ? StoreKeyState::Unavailable : StoreKeyState::NoDaemon,
                       store.result());
    }

    QByteArray encoded64 = key.toBase64();
    const QString encoded = QString::fromLatin1(encoded64);
    wipe(encoded64);
    wipe(key);
    qInfo("xmatic: store key created in the device's secrets storage");
    StoreKeyResult outcome;
    outcome.state = StoreKeyState::Available;
    outcome.key = encoded;
    return outcome;
}
