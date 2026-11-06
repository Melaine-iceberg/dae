#include "translator.h"

#include <QCoreApplication>
#include <QFile>
#include <QHash>
#include <QLocale>
#include <QQmlEngine>
#include <QTranslator>
#include <QXmlStreamReader>

namespace dae_qt {
namespace {

constexpr const char *TRANSLATION_DIRECTORY = ":/i18n/";

/// A `QTranslator` backed by the contents of a Qt Linguist `.ts` file.
///
/// Parsing the XML directly keeps `lrelease` out of the build and lets a
/// translation be reviewed as plain text. The class deliberately has no
/// `Q_OBJECT`: it only overrides virtuals, so no moc pass is needed for it.
class TsTranslator final : public QTranslator {
public:
  explicit TsTranslator(QObject *parent = nullptr) : QTranslator(parent) {}

  bool loadFile(const QString &path) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly))
      return false;

    QXmlStreamReader xml(&file);
    QString context;
    QString source;
    QString translation;
    bool insideContext = false;
    bool insideMessage = false;

    while (!xml.atEnd()) {
      xml.readNext();
      if (xml.hasError())
        break;

      if (xml.isStartElement()) {
        const auto name = xml.name();
        if (name == QStringLiteral("context")) {
          insideContext = true;
          context.clear();
        } else if (insideContext && name == QStringLiteral("name")) {
          context = xml.readElementText();
        } else if (name == QStringLiteral("message")) {
          insideMessage = true;
          source.clear();
          translation.clear();
        } else if (insideMessage && name == QStringLiteral("source")) {
          source = xml.readElementText();
        } else if (insideMessage && name == QStringLiteral("translation")) {
          translation = xml.readElementText();
        }
      } else if (xml.isEndElement()) {
        const auto name = xml.name();
        if (name == QStringLiteral("message")) {
          insideMessage = false;
          if (!source.isEmpty() && !translation.isEmpty())
            m_entries.insert(key(context, source), translation);
        } else if (name == QStringLiteral("context")) {
          insideContext = false;
        }
      }
    }
    return !m_entries.isEmpty();
  }

  QString translate(const char *context, const char *sourceText,
                    const char *disambiguation = nullptr,
                    int n = -1) const override {
    Q_UNUSED(disambiguation)
    Q_UNUSED(n)
    if (sourceText == nullptr)
      return {};
    return m_entries.value(key(QString::fromUtf8(context ? context : ""),
                               QString::fromUtf8(sourceText)));
  }

  bool isEmpty() const override { return m_entries.isEmpty(); }

private:
  static QString key(const QString &context, const QString &source) {
    return context + QChar(0x1f) + source;
  }

  QHash<QString, QString> m_entries;
};

QQmlEngine *g_engine = nullptr;
TsTranslator *g_english = nullptr;
TsTranslator *g_target = nullptr;

/// `zh-CN` -> `:/i18n/dae_zh_CN.ts`, matching the alias `build.rs` embeds.
QString resource_for(const QString &language) {
  QString tag = language;
  tag.replace(QLatin1Char('-'), QLatin1Char('_'));
  return QString::fromLatin1(TRANSLATION_DIRECTORY) + QStringLiteral("dae_") + tag +
         QStringLiteral(".ts");
}

void drop_translator(TsTranslator *&translator) {
  if (translator == nullptr)
    return;
  QCoreApplication::removeTranslator(translator);
  delete translator;
  translator = nullptr;
}

bool install_translator(const QString &path, TsTranslator *&slot) {
  auto *translator = new TsTranslator(QCoreApplication::instance());
  if (!translator->loadFile(path)) {
    delete translator;
    return false;
  }
  QCoreApplication::installTranslator(translator);
  slot = translator;
  return true;
}

QString resolve(const QString &preference) {
  if (preference == QStringLiteral("en") || preference == QStringLiteral("zh-CN") ||
      preference == QStringLiteral("zh-TW"))
    return preference;

  // The system locale picks the Chinese script; everything else is English.
  const QLocale locale = QLocale::system();
  if (locale.script() == QLocale::TraditionalChineseScript)
    return QStringLiteral("zh-TW");
  if (locale.name().startsWith(QStringLiteral("zh"), Qt::CaseInsensitive))
    return QStringLiteral("zh-CN");
  return QStringLiteral("en");
}

} // namespace

void dae_qt_i18n_attach_engine(std::size_t engine) {
  g_engine = qobject_cast<QQmlEngine *>(reinterpret_cast<QObject *>(engine));
}

bool dae_qt_i18n_apply(const QString &language) {
  drop_translator(g_target);
  drop_translator(g_english);

  const QString resolved = resolve(language);
  bool installed = false;

  // Chinese is the source language, so it installs nothing: a string the
  // source already has needs no lookup. Every other language gets English
  // first and the requested one on top — Qt searches the most recently
  // installed translator first, so a key the translation lacks reads in
  // English instead of falling back to Chinese.
  if (resolved != QStringLiteral("zh-CN")) {
    install_translator(resource_for(QStringLiteral("en")), g_english);
    installed = resolved == QStringLiteral("en")
                    ? g_english != nullptr
                    : install_translator(resource_for(resolved), g_target);
  }

  // `installTranslator` posts LanguageChange to the application, but a QML
  // binding that was already evaluated keeps its cached result until the
  // engine is told to retranslate. Doing it explicitly makes a switch from the
  // settings panel visible before the panel's own click handler returns.
  if (g_engine != nullptr)
    g_engine->retranslate();

  return installed;
}

QString dae_qt_i18n_system_language() { return resolve(QStringLiteral("system")); }

} // namespace dae_qt
