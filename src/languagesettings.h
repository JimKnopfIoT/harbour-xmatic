#ifndef LANGUAGESETTINGS_H
#define LANGUAGESETTINGS_H

#include <QObject>
#include <QString>
#include <QVariantList>

class QGuiApplication;

/// The UI language, chosen instead of followed. Empty means "follow the
/// device"; anything else overrides, English included - hence an `en` catalogue.
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

    /// Installs the chosen catalogue. After the view exists: the translator
    /// installed last is asked first, so this comes after libsailfishapp's.
    static void applyTo(QGuiApplication *app);

signals:
    void changed();

private:
    QString m_code;
};

#endif // LANGUAGESETTINGS_H
