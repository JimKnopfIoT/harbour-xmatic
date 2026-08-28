// The store key lives in Sailfish Secrets: an encrypted collection unlocked
// with the device lock, readable only by this app (OwnerOnlyMode). The key
// itself is 32 random bytes minted once on this device — it is a local
// encryption key for the stores and session file, not an account credential,
// so it survives logouts and is shared by consecutive sessions.
//
// The request choreography (create collection, store, read, each blocking on
// waitForFinished) follows the platform's documented pattern and is proven in
// the wild with these exact plugin and lock semantics on the same OS releases.

#include "secretskeeper.h"

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

/// Overwrites a buffer so that the write survives the optimiser.
///
/// `QByteArray::fill` is a memset on a buffer that is about to die, and a
/// compiler may drop a write nobody reads afterwards - which is the definition
/// of wiping a secret. Writing through a volatile pointer is the portable way
/// to say that the write itself is the point. `data()` also detaches, so this
/// is the array's own memory and not a copy somebody else still holds.
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
    // The lists that name people are encrypted under the same key and can be
    // the only thing on disk: somebody who allowed a caller and then signed
    // out has a `private.json` and neither a store marker nor a session. Minting
    // a fresh key over it makes it unreadable for good, with no way back in the
    // interface.
    if (QFile::exists(dataDirectory + QStringLiteral("/private.json"))) {
        return true;
    }
    QFile session(dataDirectory + QStringLiteral("/session.json"));
    if (!session.exists()) {
        return false;
    }
    if (!session.open(QIODevice::ReadOnly)) {
        // It is there and cannot be read. That is not "nothing is encrypted" -
        // the answer to this question decides whether a new key is put over the
        // old one, and a test whose wrong answer destroys data fails towards
        // doing nothing. Same rule as `session::load` in the core.
        return true;
    }
    // The envelope the core writes starts with its one top-level key; a
    // plaintext session starts with the homeserver. Only the shape is looked
    // at, the bytes are dropped right away.
    QByteArray head = session.read(64);
    const bool envelope = head.contains("\"encrypted\"");
    wipe(head);
    return envelope;
}

QString obtainStoreKey(const QString &dataDirectory)
{
    SecretManager manager;

    // The common case first: the key already exists. System interaction is
    // allowed so secretsd can run its device-lock authentication (a system
    // dialog, once per boot); without it a locked collection is a plain
    // failure.
    {
        StoredSecretRequest read;
        read.setManager(&manager);
        read.setIdentifier(keyIdentifier());
        read.setUserInteractionMode(SecretManager::SystemInteraction);
        read.startRequest();
        read.waitForFinished();
        const Result result = read.result();
        if (result.code() == Result::Succeeded) {
            QByteArray data = read.secret().data();
            if (data.size() == 32) {
                // The base64 is wiped too: `toBase64()` builds a second buffer
                // holding the same key in another alphabet, and wiping only
                // the first one left the key in memory anyway.
                QByteArray encoded64 = data.toBase64();
                const QString encoded = QString::fromLatin1(encoded64);
                wipe(encoded64);
                wipe(data);
                qInfo("xmatic: store key loaded from the device's secrets storage");
                return encoded;
            }
            // A key of the wrong size cannot have encrypted anything: it is
            // not the key any store was written under, so replacing it loses
            // nothing.
            qWarning("xmatic: stored key has the wrong size, creating a new one");
        } else if (encryptedDataPresent(dataDirectory)) {
            // Something on disk was written under a key, and this read did
            // not deliver it. Whatever the reason — collection locked, user
            // dismissed the dialog, daemon down — minting a new key now would
            // replace the one that data needs. The core reports the session
            // as locked and the user retries; this line says why.
            qWarning("xmatic: store key not available (%d: %s); encrypted data waits for it",
                     static_cast<int>(result.errorCode()),
                     qPrintable(result.errorMessage()));
            return QString();
        } else {
            qInfo("xmatic: no store key yet (%d: %s), creating one",
                  static_cast<int>(result.errorCode()),
                  qPrintable(result.errorMessage()));
        }
    }

    // First run (or the collection is gone): create the collection, then the
    // key. Creation failing because the collection already exists is fine —
    // the store below decides whether the whole path works.
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
        return QString();
    }

    Secret secret(keyIdentifier());
    secret.setData(key);

    StoreSecretRequest store;
    store.setManager(&manager);
    store.setSecretStorageType(StoreSecretRequest::CollectionSecret);
    store.setSecret(secret);
    store.setUserInteractionMode(SecretManager::SystemInteraction);
    store.startRequest();
    store.waitForFinished();

    if (store.result().code() != Result::Succeeded) {
        // The error string comes from secretsd and carries no secret material.
        qWarning("xmatic: secrets storage unavailable (%d: %s); stores stay unencrypted",
                 static_cast<int>(store.result().errorCode()),
                 qPrintable(store.result().errorMessage()));
        wipe(key);
        return QString();
    }

    QByteArray encoded64 = key.toBase64();
    const QString encoded = QString::fromLatin1(encoded64);
    wipe(encoded64);
    wipe(key);
    qInfo("xmatic: store key created in the device's secrets storage");
    return encoded;
}
