#include "languagesettings.h"

#include <QGuiApplication>
#include <QSettings>
#include <QStandardPaths>
#include <QTranslator>
#include <QVariantMap>

namespace {

// Same file and same reasoning as the appearance settings: UserScope would
// write one level above the app's config directory, where Sailjail blocks it.
QString settingsPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation)
           + QStringLiteral("/settings.conf");
}

QString storedCode()
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/language")).toString();
}

// Endonyms: a language is named in itself, so the list stays readable to
// someone who cannot read the current UI language — which is the situation
// this page exists for.
const char *const kLanguages[][2] = {
    { "bg", "Български" }, { "cs", "Čeština" },   { "da", "Dansk" },
    { "de", "Deutsch" },   { "el", "Ελληνικά" },  { "en", "English" },
    { "es", "Español" },   { "et", "Eesti" },     { "fi", "Suomi" },
    { "fr", "Français" },  { "ga", "Gaeilge" },   { "hr", "Hrvatski" },
    { "hu", "Magyar" },    { "is", "Íslenska" },  { "it", "Italiano" },
    { "lt", "Lietuvių" },  { "lv", "Latviešu" },  { "mt", "Malti" },
    { "nb", "Norsk bokmål" }, { "nl", "Nederlands" }, { "pl", "Polski" },
    { "pt", "Português" }, { "ro", "Română" },    { "ru", "Русский" },
    { "sk", "Slovenčina" }, { "sl", "Slovenščina" }, { "sv", "Svenska" },
};

} // namespace

LanguageSettings::LanguageSettings(QObject *parent)
    : QObject(parent)
    , m_code(storedCode())
{
}

void LanguageSettings::setCode(const QString &code)
{
    if (code == m_code) {
        return;
    }
    m_code = code;

    QSettings settings(settingsPath(), QSettings::IniFormat);
    settings.setValue(QStringLiteral("ui/language"), code);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: language setting could not be stored (%d)",
                 static_cast<int>(settings.status()));
    }

    // Not applied here: Qt 5.6 has no way to retranslate a loaded QML tree
    // (QQmlEngine::retranslate arrived in 5.10), so the choice takes effect at
    // the next start. The page says so.
    emit changed();
}

QVariantList LanguageSettings::available() const
{
    QVariantList list;
    QVariantMap automatic;
    automatic.insert(QStringLiteral("code"), QString());
    automatic.insert(QStringLiteral("name"), tr("Follow the device"));
    list.append(automatic);

    for (const auto &entry : kLanguages) {
        QVariantMap language;
        language.insert(QStringLiteral("code"), QString::fromLatin1(entry[0]));
        language.insert(QStringLiteral("name"), QString::fromUtf8(entry[1]));
        list.append(language);
    }
    return list;
}

void LanguageSettings::applyTo(QGuiApplication *app)
{
    const QString code = storedCode();
    if (code.isEmpty()) {
        return;
    }

    QTranslator *translator = new QTranslator(app);
    const QString name = QStringLiteral("harbour-xmatic-") + code;
    if (translator->load(name, QStringLiteral("/usr/share/harbour-xmatic/translations"))) {
        app->installTranslator(translator);
    } else {
        qWarning("xmatic: no catalogue for the chosen language; following the device");
        delete translator;
    }
}
