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
#include <QFontMetricsF>
#include <QHash>
#include <QMetaObject>
#include <QObject>
#include <QPointer>
#include <QQuickPaintedItem>
#include <QString>
#include <QStringList>
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
    /// Copies the highlighted cells, if any, to the system clipboard.
    void copySelection();

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
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void mouseDoubleClickEvent(QMouseEvent* event) override;
    void geometryChange(const QRectF& newGeometry, const QRectF& oldGeometry) override;
    void focusInEvent(QFocusEvent* event) override;
    void focusOutEvent(QFocusEvent* event) override;

private slots:
    /// Re-sends the grid size after the shell starts; the resize attempted
    /// before `start()` had no PTY behind it.
    void resetSize();

private:
    static constexpr quint8 FlagBold = 1 << 0;
    static constexpr quint8 FlagItalic = 1 << 1;
    static constexpr quint8 FlagUnderline = 1 << 2;
    static constexpr quint8 FlagStrikeout = 1 << 3;
    static constexpr quint8 FlagInverse = 1 << 4;
    static constexpr quint8 FlagHidden = 1 << 5;
    static constexpr quint8 FlagWide = 1 << 6;
    static constexpr quint8 FlagSpacer = 1 << 7;

    bool decodeFrame(const QByteArray& frame);
    void measureCells();
    void applySizeToSession();
    void sendBytes(const QByteArray& bytes);
    void sendText(const QString& text);
    void paste();
    QColor colour(QRgb argb) const;
    QFont styleFont(quint8 flags) const;
    QFont cellFont(quint8 flags, const QString& family) const;
    const TerminalCell& cellAt(int row, int column) const;

    void selectionBounds(int& firstRow, int& firstColumn, int& lastRow,
                         int& lastColumn) const;
    bool hasSelection() const { return m_hasSelection; }
    void clearSelection();
    void extendSelection(int row, int column);
    void selectWordAt(int row, int column);
    QString selectedText() const;

    bool scrollbarVisible() const;
    QRectF scrollbarTrack() const;
    QRectF scrollbarThumb() const;
    void scrollToOffset(int offset);
    void updateScrollbarDrag(const QPointF& position);

    /// PUA codepoints are not part of the OS font-fallback chain, so a glyph
    /// missing from the primary font has to be looked up across installed
    /// families (Nerd Fonts first) and cached.
    static bool privateUse(quint32 codepoint);
    QString fallbackFamily(quint32 codepoint);

    QPointer<QObject> m_session;
    QMetaObject::Connection m_updateConnection;
    QMetaObject::Connection m_runningConnection;
    bool m_dark = true;
    QFont m_font;
    QFontMetricsF m_primaryMetrics{m_font};
    QString m_fontFamily;
    QStringList m_fallbackCandidates;
    QHash<quint32, QString> m_fallbackFamilies;
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
    quint32 m_historySize = 0;
    bool m_selecting = false;
    bool m_hasSelection = false;
    int m_selAnchorRow = 0;
    int m_selAnchorColumn = 0;
    int m_selHeadRow = 0;
    int m_selHeadColumn = 0;
    bool m_scrollbarDrag = false;
    qreal m_scrollbarGrabOffset = 0.0;
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
