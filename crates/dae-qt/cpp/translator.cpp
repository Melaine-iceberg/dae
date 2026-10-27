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

constexpr const char *ENGLISH_RESOURCE = ":/i18n/dae_en.ts";

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
TsTranslator *g_translator = nullptr;

QString resolve(const QString &preference) {
  if (preference == QStringLiteral("en"))
    return QStringLiteral("en");
  if (preference == QStringLiteral("zh-CN"))
    return QStringLiteral("zh-CN");

  const QLocale locale = QLocale::system();
  const QString name = locale.name();
  if (name.startsWith(QStringLiteral("zh"), Qt::CaseInsensitive))
    return QStringLiteral("zh-CN");
  if (name.startsWith(QStringLiteral("en"), Qt::CaseInsensitive))
    return QStringLiteral("en");
  return QStringLiteral("en");
}

} // namespace

void dae_qt_i18n_attach_engine(std::size_t engine) {
  g_engine = qobject_cast<QQmlEngine *>(reinterpret_cast<QObject *>(engine));
}

bool dae_qt_i18n_apply(const QString &language) {
  if (g_translator != nullptr) {
    QCoreApplication::removeTranslator(g_translator);
    delete g_translator;
    g_translator = nullptr;
  }

  const QString resolved = resolve(language);
  bool installed = false;
  if (resolved == QStringLiteral("en")) {
    auto *translator = new TsTranslator(QCoreApplication::instance());
    if (translator->loadFile(QString::fromLatin1(ENGLISH_RESOURCE))) {
      g_translator = translator;
      QCoreApplication::installTranslator(translator);
      installed = true;
    } else {
      delete translator;
    }
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
