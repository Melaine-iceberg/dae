// The terminal's self-drawn grid.
//
// `alacritty_terminal` owns the cells in Rust; this item receives a compact
// frame (see `crates/dae-qt/src/terminal.rs`) and paints it with QPainter.
// Painting is deliberately cell-oriented: a monospaced grid, background runs,
// then text runs with one style each, so a full screen costs a handful of calls
// rather than one per character.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <QByteArray>
#include <QColor>
#include <QFont>
#include <QMetaObject>
#include <QObject>
#include <QPointer>
#include <QQuickPaintedItem>
#include <QString>
#include <QTimer>
#include <QVector>

#include <QtQml/qqml.h>

namespace dae_qt {

struct TerminalCell
{
    uint ch = ' ';
    QRgb fg = 0xffd8dee9;
    QRgb bg = 0xff14161a;
    quint8 flags = 0;
};

class TerminalItem : public QQuickPaintedItem
{
    Q_OBJECT
    Q_PROPERTY(QObject* session READ session WRITE setSession NOTIFY sessionChanged)
    Q_PROPERTY(bool dark READ dark WRITE setDark NOTIFY darkChanged)
    Q_PROPERTY(int fontSize READ fontSize WRITE setFontSize NOTIFY fontChanged)
    Q_PROPERTY(qreal lineHeight READ lineHeight WRITE setLineHeight NOTIFY fontChanged)
    Q_PROPERTY(QString fontFamily READ fontFamily WRITE setFontFamily NOTIFY fontChanged)

public:
    explicit TerminalItem(QQuickItem* parent = nullptr);

    QObject* session() const { return m_session.data(); }
    void setSession(QObject* session);

    bool dark() const { return m_dark; }
    void setDark(bool dark);

    int fontSize() const { return m_font.pixelSize(); }
    void setFontSize(int size);

    qreal lineHeight() const { return m_lineHeight; }
    void setLineHeight(qreal factor);

    QString fontFamily() const { return m_fontFamily; }
    void setFontFamily(const QString& family);

    void paint(QPainter* painter) override;

public slots:
    /// Pulls a new frame. The session's `updated` signal is connected here.
    void refresh();
    void focusTerminal() { forceActiveFocus(); }

signals:
    void sessionChanged();
    void darkChanged();
    void fontChanged();

protected:
    void keyPressEvent(QKeyEvent* event) override;
    void inputMethodEvent(QInputMethodEvent* event) override;
    QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;
    void wheelEvent(QWheelEvent* event) override;
    void mousePressEvent(QMouseEvent* event) override;
    void geometryChange(const QRectF& newGeometry, const QRectF& oldGeometry) override;
    void focusInEvent(QFocusEvent* event) override;
    void focusOutEvent(QFocusEvent* event) override;

private:
    static constexpr quint8 FlagBold = 1 << 0;
    static constexpr quint8 FlagItalic = 1 << 1;
    static constexpr quint8 FlagUnderline = 1 << 2;
    static constexpr quint8 FlagStrikeout = 1 << 3;
    static constexpr quint8 FlagInverse = 1 << 4;
    static constexpr quint8 FlagHidden = 1 << 5;
    static constexpr quint8 FlagWide = 1 << 6;
    static constexpr quint8 FlagSpacer = 1 << 7;

    void decodeFrame(const QByteArray& frame);
    void measureCells();
    void applySizeToSession();
    void sendBytes(const QByteArray& bytes);
    void sendText(const QString& text);
    void paste();
    QColor colour(QRgb argb) const;
    QFont styleFont(quint8 flags) const;
    const TerminalCell& cellAt(int row, int column) const;

    QPointer<QObject> m_session;
    QMetaObject::Connection m_updateConnection;
    bool m_dark = true;
    QFont m_font;
    QString m_fontFamily;
    qreal m_lineHeight = 1.75;
    qreal m_cellWidth = 0.0;
    qreal m_cellHeight = 0.0;
    qreal m_ascent = 0.0;
    int m_cols = 0;
    int m_rows = 0;
    int m_lastCols = 0;
    int m_lastRows = 0;
    int m_cursorCol = 0;
    int m_cursorRow = 0;
    bool m_cursorVisible = false;
    quint8 m_cursorShape = 0;
    quint32 m_displayOffset = 0;
    QVector<TerminalCell> m_cells;
    QString m_preedit;
    bool m_hasFrame = false;
    QTimer m_blink;
    bool m_blinkOn = true;
};

/// Called from `dae_qt::init` before the QML engine loads. The module name is
/// separate from `dae` because this type is a C++ QQuickItem, not a cxx-qt
/// object, and registering it through the generated plugin would require a
/// qmldir entry the generator owns.
inline void dae_qt_register_terminal_item()
{
    qmlRegisterType<TerminalItem>("dae.terminal", 1, 0, "TerminalItem");
}

} // namespace dae_qt
