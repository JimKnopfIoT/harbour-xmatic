#ifndef LANGUAGESETTINGS_H
#define LANGUAGESETTINGS_H

#include <QObject>
#include <QString>
#include <QVariantList>

class QGuiApplication;

/// The UI language, chosen instead of followed.
///
/// Empty code means "follow the device", which is what libsailfishapp does on
/// its own. Anything else overrides it — English included, which is why there
/// is an `en` catalogue although English is the source language.
class LanguageSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString code READ code WRITE setCode NOTIFY changed)
    /// `{ code, name }` per entry, name in its own language. Empty code first.
    Q_PROPERTY(QVariantList available READ available CONSTANT)

public:
    explicit LanguageSettings(QObject *parent = nullptr);

    QString code() const { return m_code; }
    void setCode(const QString &code);
    QVariantList available() const;

    /// Installs the chosen catalogue, or nothing when following the device.
    /// Call after the view exists: the translator installed last is asked
    /// first, so this has to come after libsailfishapp's own.
    static void applyTo(QGuiApplication *app);

signals:
    void changed();

private:
    QString m_code;
};

#endif // LANGUAGESETTINGS_H
