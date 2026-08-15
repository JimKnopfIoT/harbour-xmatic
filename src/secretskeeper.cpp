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

QString obtainStoreKey()
{
    SecretManager manager;

    // The common case first: the key already exists.
    {
        StoredSecretRequest read;
        read.setManager(&manager);
        read.setIdentifier(keyIdentifier());
        read.startRequest();
        read.waitForFinished();
        if (read.result().code() == Result::Succeeded) {
            QByteArray data = read.secret().data();
            if (data.size() == 32) {
                const QString encoded = QString::fromLatin1(data.toBase64());
                data.fill('\0');
                qInfo("xmatic: store key loaded from the device's secrets storage");
                return encoded;
            }
            qWarning("xmatic: stored key has the wrong size, creating a new one");
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
        create.startRequest();
        create.waitForFinished();
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
    store.startRequest();
    store.waitForFinished();

    if (store.result().code() != Result::Succeeded) {
        // The one line that says why an install runs unencrypted. The error
        // string comes from secretsd and carries no secret material.
        qWarning("xmatic: secrets storage unavailable (%s); stores stay unencrypted",
                 qPrintable(store.result().errorMessage()));
        key.fill('\0');
        return QString();
    }

    const QString encoded = QString::fromLatin1(key.toBase64());
    key.fill('\0');
    qInfo("xmatic: store key created in the device's secrets storage");
    return encoded;
}
