#include "crosssigning.h"

CrossSigning::CrossSigning(QObject *parent)
    : QObject(parent)
{
}

void CrossSigning::setUp(const QString &password)
{
    if (password.isEmpty()) {
        emit refused(tr("Enter your password first."));
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("password"), password);
    emit commandReady(QStringLiteral("crossSigning.setup"), arguments);
}

void CrossSigning::noteNeedsPassword()
{
    setNeedsPassword(true);
}

bool CrossSigning::deliver(const QString &command, const QJsonObject &data)
{
    if (command != QLatin1String("crossSigning.setup")) {
        return false;
    }
    setNeedsPassword(false);
    if (!data.value(QStringLiteral("created")).toBool(true)) {
        emit alreadyThere();
    }
    return true;
}

void CrossSigning::reset()
{
    setNeedsPassword(false);
}

void CrossSigning::setNeedsPassword(bool needs)
{
    if (needs != m_needsPassword) {
        m_needsPassword = needs;
        emit changed();
    }
}
