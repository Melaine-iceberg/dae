// The terminal's self-drawn grid.
//
// `libghostty-vt` owns the cells in Rust; this item receives a compact
// frame (see `crates/dae-qt/src/terminal.rs`) and paints it with QPainter.
// Painting is deliberately cell-oriented: a monospaced grid, background runs,
// then text runs with one style each, so a full screen costs a handful of calls
// rather than one per character.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <QRegularExpression>

#include <QElapsedTimer>
#include <QFont>
#include <QFontMetricsF>
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
    QRgb fg = 0xfff7f7f9;
    QRgb bg = 0xff101216;
    quint8 flags = 0;
};

/// One search-highlight rectangle in grid coordinates, decoded from the
/// session's `searchRects` payload.
struct SearchRect
{
    quint16 row = 0;
    quint16 column = 0;
    quint16 columns = 0;
};

class TerminalItem : public QQuickPaintedItem
{
    Q_OBJECT
    Q_PROPERTY(QObject* session READ session WRITE setSession NOTIFY sessionChanged)
    Q_PROPERTY(bool dark READ dark WRITE setDark NOTIFY darkChanged)
    /// Copy to the clipboard as soon as the mouse finishes a selection,
    /// the way Windows Terminal's copyOnSelect does. The panel binds it
    /// to the matching ExplorerPrefs entry.
    Q_PROPERTY(bool copyOnSelect READ copyOnSelect WRITE setCopyOnSelect NOTIFY copyOnSelectChanged)
    Q_PROPERTY(int fontSize READ fontSize WRITE setFontSize NOTIFY fontChanged)
    Q_PROPERTY(qreal lineHeight READ lineHeight WRITE setLineHeight NOTIFY fontChanged)
    Q_PROPERTY(QString fontFamily READ fontFamily WRITE setFontFamily NOTIFY fontChanged)
    /// The grid size this item can show. The panel starts the shell with it so
    /// the PTY never has to be resized right after the first prompt.
    Q_PROPERTY(int columns READ columns NOTIFY gridSizeChanged)
    Q_PROPERTY(int rows READ rows NOTIFY gridSizeChanged)

public:
    explicit TerminalItem(QQuickItem* parent = nullptr);

    QObject* session() const { return m_session.data(); }
    void setSession(QObject* session);

    bool dark() const { return m_dark; }
    void setDark(bool dark);

    bool copyOnSelect() const { return m_copyOnSelect; }
    void setCopyOnSelect(bool value);

    int fontSize() const { return m_font.pixelSize(); }
    void setFontSize(int size);

    qreal lineHeight() const { return m_lineHeight; }
    void setLineHeight(qreal factor);

    QString fontFamily() const { return m_fontFamily; }
    void setFontFamily(const QString& family);

    int columns() const { return qMax(1, int(width() / m_cellWidth)); }
    int rows() const { return qMax(1, int(height() / m_cellHeight)); }

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
    void gridSizeChanged();
    /// Ctrl+F was pressed inside the terminal; the panel opens its find
    /// bar. Emitted instead of sending the form-feed byte to the shell.
    void searchRequested();
    void copyOnSelectChanged();
    /// Ctrl+Click landed on something that looks like a path or URL. The
    /// panel resolves it against the session's directory and opens it;
    /// `line`/`column` are the `:line:col` suffix when one was present.
    void linkActivated(const QString& target, int line, int column);

protected:
    void keyPressEvent(QKeyEvent* event) override;
    void keyReleaseEvent(QKeyEvent* event) override;
    void inputMethodEvent(QInputMethodEvent* event) override;
    QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;
    void wheelEvent(QWheelEvent* event) override;
    void mousePressEvent(QMouseEvent* event) override;
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void mouseDoubleClickEvent(QMouseEvent* event) override;
    void hoverMoveEvent(QHoverEvent* event) override;
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

    bool decodeFrame(const QByteArray& frame);
    /// Pulls and decodes the session's `searchRects` payload so `paint`
    /// can highlight the matches on screen.
    void pullSearchRects();
    /// Matches the clicked row against the path/URL patterns and emits
    /// [`linkActivated`] on a hit.
    void openLinkAt(const QPointF& position);
    void measureCells();
    void applySizeToSession();
    void sendBytes(const QByteArray& bytes);
    void sendText(const QString& text);
    bool sendKeyEvent(int action, QKeyEvent* event);
    bool sendMouseEvent(int action, int button, Qt::KeyboardModifiers modifiers,
                        const QPointF& position, bool anyPressed);
    bool isMouseTracking() const;
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
    bool m_dark = true;
    QFont m_font;
    QFontMetricsF m_primaryMetrics{m_font};
    QString m_fontFamily;
    QStringList m_fallbackCandidates;
    QHash<quint32, QString> m_fallbackFamilies;
    qreal m_lineHeight = 1.4;
    qreal m_cellWidth = 0.0;
    qreal m_cellHeight = 0.0;
    qreal m_ascent = 0.0;
    /// Descent of the primary face. The cursor and preedit are drawn at the
    /// glyph height (`ascent + descent`), not the full spaced cell, so a
    /// generous line height does not inflate them the way Zed's terminal
    /// does not.
    qreal m_descent = 0.0;
    int m_cols = 0;
    int m_rows = 0;
    int m_lastCols = 0;
    int m_lastRows = 0;
    int m_cursorCol = 0;
    int m_cursorRow = 0;
    bool m_cursorVisible = false;
    bool m_copyOnSelect = false;
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
    QVector<SearchRect> m_searchRects;
    QString m_preedit;
    bool m_hasFrame = false;
    QTimer m_blink;
    bool m_blinkOn = true;
    QElapsedTimer m_lastRefresh;
    bool m_refreshQueued = false;
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
