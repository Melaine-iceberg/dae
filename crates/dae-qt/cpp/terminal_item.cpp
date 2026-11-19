#include "terminal_item.h"

#include <QClipboard>
#include <QFontDatabase>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QInputMethod>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QWheelEvent>

#include <cstring>

namespace dae_qt {

namespace {

constexpr int FrameHeader = 4 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 4 + 4 + 1 + 2;
constexpr int FrameCell = 4 + 4 + 4 + 1;
constexpr qreal ScrollbarWidth = 6.0;
constexpr qreal ScrollbarMargin = 3.0;
constexpr qreal ScrollbarMinThumb = 24.0;

quint16 readU16(const uchar* data)
{
    return quint16(data[0]) | (quint16(data[1]) << 8);
}

/// The fixed-pitch faces in resolution order: the bundled Mona Sans Mono VF,
/// then Symbols Nerd Font Mono (private-use icon glyphs only — the ones a
/// powerline/oh-my-posh prompt draws), then the platform monos. Qt walks the
/// list per glyph, so Chinese and icons reach a face that has them while every
/// cell that does have a Mona glyph keeps its advance.
QStringList bundledMonoFamilies()
{
    return QStringList{ QStringLiteral("Mona Sans Mono VF"),
                        QStringLiteral("Mona Sans Mono"),
                        QStringLiteral("Symbols Nerd Font Mono"),
                        QStringLiteral("Cascadia Mono"),
                        QStringLiteral("Consolas"),
                        QStringLiteral("Courier New") };
}

/// Draws `text` into `rect`, squeezing it horizontally when the face that owns
/// the glyphs advances wider than the cells those glyphs occupy.
///
/// A Mona Sans Mono cell is 0.618 em; an icon face's glyph is a full em. Drawn
/// at its own advance an icon would run past the right edge of its cell and be
/// clipped there (Qt clips `drawText` to the rectangle it was given), so the
/// grid wins: the run keeps its height and baseline and gives up width. Text
/// from the cell's own face measures exactly `rect` wide, so this is a no-op
/// for every ordinary run.
void drawCellText(QPainter* painter, const QRectF& rect, const QString& text)
{
    const qreal natural = QFontMetricsF(painter->font()).horizontalAdvance(text);
    painter->save();
    if (natural > rect.width() + 0.5) {
        const qreal left = rect.left();
        painter->translate(left, 0.0);
        painter->scale(rect.width() / natural, 1.0);
        painter->translate(-left, 0.0);
    }
    // TextDontClip: several prompt glyphs (starship's `❯`, powerline arrows)
    // carry ink past the cell's left edge, and Qt clips drawText to the
    // given rectangle — cutting the ink made `❯` read as `)`. The squeeze
    // above already keeps wide glyphs inside their cells, so removing the
    // clip only lets sub-pixel bearings bleed a hair into the neighbour.
    painter->drawText(rect, Qt::TextSingleLine | Qt::TextDontClip | Qt::AlignLeft
                                 | Qt::AlignVCenter, text);
    painter->restore();
}

quint32 readU32(const uchar* data)
{
    return quint32(data[0]) | (quint32(data[1]) << 8) | (quint32(data[2]) << 16)
        | (quint32(data[3]) << 24);
}

/// One Unicode scalar as a QString. `QChar(uint)` asserts above U+FFFF, so
/// astral characters (most emoji) must go through `fromUcs4`, which produces
/// the surrogate pair the font engine expects.
QString character(quint32 codepoint)
{
    if (codepoint <= 0xffff)
        return QString(QChar(ushort(codepoint)));
    return QString::fromUcs4(&codepoint, 1);
}

bool wordCharacter(quint32 codepoint)
{
    return QChar::isLetterOrNumber(codepoint) || codepoint == '_' || codepoint == '-'
        || codepoint == '.' || codepoint == '/' || codepoint == '\\' || codepoint == ':'
        || codepoint == '~';
}

/// The paint-side companions to the Rust palette. The grid's cells carry their
/// own colours; these are the surface behind it, the placeholder, and the
/// cursor/selection accent. All six values mirror `Theme.qml`: canvas, the
/// three ink steps, and the accent for both themes.
QColor terminalBackground(bool dark)
{
    return dark ? QColor(0x10, 0x12, 0x16) : QColor(0xfd, 0xfd, 0xfd);
}

QColor terminalPlaceholder(bool dark)
{
    return dark ? QColor(0x96, 0x98, 0x9d) : QColor(0x6b, 0x6d, 0x72);
}

QColor terminalInk(bool dark)
{
    return dark ? QColor(0xf7, 0xf7, 0xf9) : QColor(0x1c, 0x1e, 0x23);
}

QColor terminalAccent(bool dark)
{
    return dark ? QColor(0x6b, 0xca, 0xc9) : QColor(0x00, 0x7a, 0x7b);
}

} // namespace

TerminalItem::TerminalItem(QQuickItem* parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::LeftButton);
    setFlag(ItemAcceptsInputMethod, true);
    setFocus(true);
    setActiveFocusOnTab(true);

    m_font.setFamilies(bundledMonoFamilies());
    m_font.setStyleHint(QFont::Monospace);
    m_font.setFixedPitch(true);
    m_font.setPixelSize(13);
    // Mona Sans Mono VF's variable default instance is ExtraLight (wght=200),
    // and while Qt maps QFont::weight onto `wght` for a request, pinning it
    // says out loud which instance the grid is measured from. Bold cells raise
    // the same axis in `styleFont` instead of padding it with a synthetic
    // embolden.
    // The terminal paints through QPainter, not the scene graph's text path.
    // Hinted glyphs snap their stems to the pixel grid, which reads as
    // "no antialiasing" next to unhinted renderers; Zed's terminal draws
    // unhinted with grayscale coverage, so the grid asks for the same and
    // lets the rasteriser antialias every edge (supersedes the earlier
    // PreferVerticalHinting note).
    m_font.setHintingPreference(QFont::PreferNoHinting);
    measureCells();

    m_blink.setInterval(530);
    connect(&m_blink, &QTimer::timeout, this, [this]() {
        m_blinkOn = !m_blinkOn;
        update();
    });
    connect(this, &QQuickItem::activeFocusChanged, this, [this]() {
        if (hasActiveFocus()) {
            m_blinkOn = true;
            m_blink.start();
            QGuiApplication::inputMethod()->update(Qt::ImQueryAll);
        } else {
            m_blink.stop();
            m_blinkOn = true;
        }
        update();
    });
}

void TerminalItem::setSession(QObject* session)
{
    if (m_session.data() == session)
        return;

    if (m_updateConnection)
        QObject::disconnect(m_updateConnection);
    m_session = session;
    if (m_session) {
        m_updateConnection = connect(m_session.data(), SIGNAL(updated()), this, SLOT(refresh()));
    }
    emit sessionChanged();
    // The previous session's grid size says nothing about this one; the size
    // is re-sent so a session bound after a tab switch still learns it.
    m_lastCols = -1;
    m_lastRows = -1;
    clearSelection();
    applySizeToSession();
    refresh();
}

void TerminalItem::setDark(bool dark)
{
    if (m_dark == dark)
        return;
    m_dark = dark;
    emit darkChanged();
    // The cells carry their colours, so the light/dark palette is picked when
    // the frame is pulled — a repaint alone would keep the old theme's cells.
    refresh();
}

void TerminalItem::setCopyOnSelect(bool value)
{
    if (m_copyOnSelect == value)
        return;
    m_copyOnSelect = value;
    emit copyOnSelectChanged();
}

void TerminalItem::setFontSize(int size)
{
    const int clamped = qBound(6, size, 32);
    if (m_font.pixelSize() == clamped)
        return;
    m_font.setPixelSize(clamped);
    m_fallbackFamilies.clear();
    m_fallbackCandidates.clear();
    measureCells();
    applySizeToSession();
    emit fontChanged();
    update();
}

void TerminalItem::setLineHeight(qreal factor)
{
    const qreal clamped = qBound(0.8, factor, 3.0);
    if (qFuzzyCompare(m_lineHeight, clamped))
        return;
    m_lineHeight = clamped;
    measureCells();
    applySizeToSession();
    emit fontChanged();
    update();
}

void TerminalItem::setFontFamily(const QString& family)
{
    const QString trimmed = family.trimmed();
    if (m_fontFamily == trimmed)
        return;
    m_fontFamily = trimmed;
    if (trimmed.isEmpty() || bundledMonoFamilies().contains(trimmed))
        m_font.setFamilies(bundledMonoFamilies());
    else
        // A chosen family still gets the icon face behind it: a prompt's
        // private-use glyphs are almost never in an ordinary mono, and a cell
        // that cannot be drawn is worse than one drawn from a fallback.
        m_font.setFamilies({ trimmed, QStringLiteral("Symbols Nerd Font Mono") });
    m_fallbackFamilies.clear();
    m_fallbackCandidates.clear();
    measureCells();
    applySizeToSession();
    emit fontChanged();
    update();
}

void TerminalItem::measureCells()
{
    m_primaryMetrics = QFontMetricsF(m_font);
    const QFontMetricsF& metrics = m_primaryMetrics;
    m_cellWidth = qMax(1.0, metrics.horizontalAdvance(QLatin1Char('M')));
    m_cellHeight = qMax(1.0, metrics.lineSpacing() * m_lineHeight);
    m_ascent = metrics.ascent();
    m_descent = metrics.descent();
}

void TerminalItem::applySizeToSession()
{
    if (m_cellWidth <= 0.0 || m_cellHeight <= 0.0)
        measureCells();
    const int cols = qMax(1, int(width() / m_cellWidth));
    const int rows = qMax(1, int(height() / m_cellHeight));
    if (cols == m_lastCols && rows == m_lastRows)
        return;
    m_lastCols = cols;
    m_lastRows = rows;
    emit gridSizeChanged();
    if (m_session) {
        QMetaObject::invokeMethod(m_session.data(), "resize", Q_ARG(int, cols), Q_ARG(int, rows));
    }
}

void TerminalItem::refresh()
{
    // Output wakes can outpace the compositor. Pulling and decoding a frame on
    // every delivered wake keeps the GUI thread busy for the whole burst, so
    // cap the pull at one per frame and let the trailing wake draw the final
    // state instead of dropping it.
    constexpr qint64 FrameMs = 16;
    if (m_lastRefresh.isValid()) {
        const qint64 since = m_lastRefresh.elapsed();
        if (since < FrameMs) {
            if (!m_refreshQueued) {
                m_refreshQueued = true;
                QTimer::singleShot(int(FrameMs - since), Qt::PreciseTimer, this, [this] {
                    m_refreshQueued = false;
                    refresh();
                });
            }
            return;
        }
    }
    m_lastRefresh.restart();

    if (!m_session) {
        m_hasFrame = false;
        m_cells.clear();
        m_cols = 0;
        m_rows = 0;
        m_historySize = 0;
        m_displayOffset = 0;
        m_searchRects.clear();
        clearSelection();
        update();
        return;
    }

    pullSearchRects();

    QByteArray frame;
    const bool pulled = QMetaObject::invokeMethod(m_session.data(), "snapshot",
                                                  Q_RETURN_ARG(QByteArray, frame),
                                                  Q_ARG(bool, m_dark));
    // A session that has not produced a frame yet (starting, or just switched
    // to) must not keep showing the previous tab's grid.
    if (!pulled || !decodeFrame(frame)) {
        m_hasFrame = false;
        m_cells.clear();
        m_cols = 0;
        m_rows = 0;
        m_historySize = 0;
        m_displayOffset = 0;
        m_cursorVisible = false;
        clearSelection();
    }
    // A full frame repaints everything; a shift frame moved every row, so it
    // does too — its savings were on the bridge, not in the damage region.
    update();
}

/// The session's `searchRects` payload: `u16 count`, then per rectangle
/// `row`/`column`/`columns` as `u16` each, little endian. An empty payload
/// (no session, no search) clears the highlights.
void TerminalItem::pullSearchRects()
{
    m_searchRects.clear();
    QByteArray payload;
    if (!QMetaObject::invokeMethod(m_session.data(), "searchRects",
                                   Q_RETURN_ARG(QByteArray, payload))
        || payload.size() < 2) {
        return;
    }
    const auto* data = reinterpret_cast<const uchar*>(payload.constData());
    const int count = readU16(data);
    if (payload.size() < 2 + qsizetype(count) * 6)
        return;
    m_searchRects.reserve(count);
    for (int index = 0; index < count; ++index) {
        const uchar* rect = data + 2 + index * 6;
        m_searchRects.append(SearchRect{ readU16(rect), readU16(rect + 2), readU16(rect + 4) });
    }
}

bool TerminalItem::decodeFrame(const QByteArray& frame)
{
    if (frame.size() < FrameHeader)
        return false;
    const auto* data = reinterpret_cast<const uchar*>(frame.constData());
    if (std::memcmp(data, "DAET", 4) != 0 || data[4] != 3)
        return false;

    const int cols = readU16(data + 5);
    const int rows = readU16(data + 7);
    if (cols <= 0 || rows <= 0 || cols > 4096 || rows > 4096)
        return false;
    const int kind = data[23];
    const int count = readU16(data + 24);

    if (kind == 1) {
        // Shift frame: the viewport scrolled up `count` lines and the
        // `count` rows that follow are the new bottom lines, top to bottom.
        const qsizetype expected =
            qsizetype(FrameHeader) + qsizetype(count) * cols * FrameCell;
        if (count <= 0 || count >= rows || frame.size() < expected)
            return false;
        if (cols != m_cols || rows != m_rows || !m_hasFrame)
            return false; // the shadow grid cannot be moved at new dimensions
        const qsizetype grid = qsizetype(cols) * rows;
        std::memmove(m_cells.data(), m_cells.data() + qsizetype(count) * cols,
                     (grid - qsizetype(count) * cols) * sizeof(TerminalCell));
        const uchar* cell = data + FrameHeader;
        TerminalCell* target = m_cells.data() + grid - qsizetype(count) * cols;
        for (int index = 0; index < count * cols; ++index) {
            target[index].ch = readU32(cell);
            target[index].fg = readU32(cell + 4);
            target[index].bg = readU32(cell + 8);
            target[index].flags = cell[12];
            cell += FrameCell;
        }
        clearSelection();
    } else {
        if (kind != 0 || count != 0)
            return false;
        if (frame.size() < FrameHeader + qsizetype(cols) * rows * FrameCell)
            return false;
        if (cols != m_cols || rows != m_rows)
            clearSelection();
        m_cols = cols;
        m_rows = rows;
        m_cells.resize(qsizetype(cols) * rows);
        const uchar* cell = data + FrameHeader;
        for (int index = 0; index < cols * rows; ++index) {
            TerminalCell& target = m_cells[index];
            target.ch = readU32(cell);
            target.fg = readU32(cell + 4);
            target.bg = readU32(cell + 8);
            target.flags = cell[12];
            cell += FrameCell;
        }
    }

    m_cursorCol = readU16(data + 9);
    m_cursorRow = readU16(data + 11);
    m_cursorVisible = data[13] != 0;
    m_cursorShape = data[14];
    m_displayOffset = readU32(data + 15);
    m_historySize = readU32(data + 19);

    m_hasFrame = true;
    return true;
}


void TerminalItem::paint(QPainter* painter)
{
    painter->fillRect(boundingRect(), terminalBackground(m_dark));

    if (!m_hasFrame || m_cols <= 0 || m_rows <= 0) {
        painter->setPen(terminalPlaceholder(m_dark));
        painter->drawText(boundingRect(), Qt::AlignCenter,
                          m_session ? QStringLiteral("终端") : QStringLiteral("终端未启动"));
        return;
    }

    // Backgrounds first, merged into horizontal runs.
    for (int row = 0; row < m_rows; ++row) {
        const qreal y = row * m_cellHeight;
        int column = 0;
        while (column < m_cols) {
            const QRgb background = cellAt(row, column).bg;
            const int start = column;
            while (column < m_cols && cellAt(row, column).bg == background)
                ++column;
            painter->fillRect(QRectF(start * m_cellWidth, y, (column - start) * m_cellWidth,
                                     m_cellHeight),
                              QColor::fromRgba(background));
        }
    }

    if (hasSelection()) {
        int firstRow, firstColumn, lastRow, lastColumn;
        selectionBounds(firstRow, firstColumn, lastRow, lastColumn);
        QColor highlight = terminalAccent(m_dark);
        highlight.setAlpha(70);
        for (int row = firstRow; row <= lastRow && row < m_rows; ++row) {
            if (row < 0)
                continue;
            const int start = row == firstRow ? firstColumn : 0;
            const int end = row == lastRow ? lastColumn : m_cols - 1;
            painter->fillRect(QRectF(start * m_cellWidth, row * m_cellHeight,
                                     (end - start + 1) * m_cellWidth, m_cellHeight),
                              highlight);
        }
    }

    // Search highlights sit under the text as well, lighter than the
    // selection so both remain readable when they overlap.
    if (!m_searchRects.isEmpty()) {
        QColor match = terminalAccent(m_dark);
        match.setAlpha(40);
        for (const SearchRect& rect : m_searchRects) {
            if (rect.row >= m_rows || rect.column >= m_cols)
                continue;
            const int columns = qMin(int(rect.columns), m_cols - rect.column);
            painter->fillRect(QRectF(rect.column * m_cellWidth, rect.row * m_cellHeight,
                                     columns * m_cellWidth, m_cellHeight),
                              match);
        }
    }

    // Then text, one drawText per style/colour/font run.
    const quint8 styleMask = FlagBold | FlagItalic | FlagUnderline | FlagStrikeout | FlagHidden;
    for (int row = 0; row < m_rows; ++row) {
        const qreal y = row * m_cellHeight;
        int column = 0;
        while (column < m_cols) {
            const TerminalCell& first = cellAt(row, column);
            if (first.flags & FlagSpacer) {
                ++column;
                continue;
            }
            const QString firstFamily = fallbackFamily(first.ch);
            if (first.flags & FlagWide) {
                painter->setFont(cellFont(first.flags, firstFamily));
                painter->setPen(colour(first.fg));
                drawCellText(painter,
                             QRectF(column * m_cellWidth, y, m_cellWidth * 2.0, m_cellHeight),
                             character(first.ch));
                column += 2;
                continue;
            }

            const quint8 style = first.flags & styleMask;
            const QRgb foreground = first.fg;
            const int start = column;
            QString text;
            while (column < m_cols) {
                const TerminalCell& cell = cellAt(row, column);
                if (cell.flags & (FlagSpacer | FlagWide))
                    break;
                if ((cell.flags & styleMask) != style || cell.fg != foreground)
                    break;
                if (fallbackFamily(cell.ch) != firstFamily)
                    break;
                text.append(character(cell.ch));
                ++column;
            }
            if (text.isEmpty()) {
                ++column;
                continue;
            }
            painter->setFont(cellFont(style, firstFamily));
            painter->setPen(colour(foreground));
            drawCellText(painter,
                         QRectF(start * m_cellWidth, y, (column - start) * m_cellWidth, m_cellHeight),
                         text);
        }
    }

    if (!m_preedit.isEmpty()) {
        const QRectF at(m_cursorCol * m_cellWidth, m_cursorRow * m_cellHeight,
                        m_cellWidth * qMax(1, m_preedit.size()), m_cellHeight);
        painter->setPen(terminalInk(m_dark));
        painter->drawText(at, Qt::TextSingleLine | Qt::AlignLeft | Qt::AlignVCenter, m_preedit);
        painter->drawLine(at.bottomLeft(), at.bottomRight());
    }

    const bool drawCursor = m_cursorVisible && (!hasActiveFocus() || m_blinkOn);
    if (drawCursor) {
        // The cursor covers the glyph height, not the spaced cell: with a
        // generous line height the cell is taller than the text and a
        // full-cell cursor reads as a mis-sized block. Zed draws the cursor
        // at the text height, centred in the row.
        const qreal glyphHeight = m_ascent + m_descent;
        const qreal glyphTop
            = m_cursorRow * m_cellHeight + (m_cellHeight - glyphHeight) / 2.0;
        const QRectF cursor(m_cursorCol * m_cellWidth, glyphTop, m_cellWidth, glyphHeight);
        const QColor accent = terminalAccent(m_dark);
        switch (m_cursorShape) {
        case 1: // underline
            painter->fillRect(QRectF(cursor.left(), cursor.bottom() - 2.0, cursor.width(), 2.0),
                              accent);
            break;
        case 2: // beam
            painter->fillRect(QRectF(cursor.left(), cursor.top(), 2.0, cursor.height()), accent);
            break;
        default: {
            QColor fill = accent;
            fill.setAlpha(80);
            painter->fillRect(cursor, fill);
            painter->setPen(accent);
            painter->drawRect(cursor.adjusted(0.5, 0.5, -0.5, -0.5));
            break;
        }
        }
    }

    if (scrollbarVisible()) {
        painter->setPen(Qt::NoPen);
        QColor scrollbar = terminalPlaceholder(m_dark);
        scrollbar.setAlpha(45);
        painter->setBrush(scrollbar);
        painter->drawRoundedRect(scrollbarTrack(), ScrollbarWidth / 2.0, ScrollbarWidth / 2.0);
        scrollbar.setAlpha(m_scrollbarDrag ? 200 : 130);
        painter->setBrush(scrollbar);
        painter->drawRoundedRect(scrollbarThumb(), ScrollbarWidth / 2.0, ScrollbarWidth / 2.0);
    }
}

void TerminalItem::keyPressEvent(QKeyEvent* event)
{
    if (!m_session) {
        QQuickPaintedItem::keyPressEvent(event);
        return;
    }

    const int key = event->key();
    const Qt::KeyboardModifiers modifiers = event->modifiers();
    const bool control = modifiers.testFlag(Qt::ControlModifier);
    const bool meta = modifiers.testFlag(Qt::MetaModifier);
    const bool alt = modifiers.testFlag(Qt::AltModifier);
    const bool shift = modifiers.testFlag(Qt::ShiftModifier);

    // The panel's find bar owns Ctrl+F; a terminal has no use for the
    // form-feed byte the shell would otherwise see.
    if (control && !alt && key == Qt::Key_F) {
        emit searchRequested();
        event->accept();
        return;
    }

    // A selection turns Ctrl+C into copy, the way Windows Terminal behaves;
    // without one it stays the interrupt byte below.
    if ((control && !alt && key == Qt::Key_C) || (meta && key == Qt::Key_C)
        || (control && key == Qt::Key_Insert)) {
        if (hasSelection()) {
            copySelection();
            event->accept();
            return;
        }
    }
    if (((control || meta) && key == Qt::Key_V) || (control && shift && key == Qt::Key_V)
        || (shift && key == Qt::Key_Insert)) {
        paste();
        event->accept();
        return;
    }
    // Any other key starts a fresh command; a highlight left from copying
    // would otherwise keep painting over the output that follows. Modifier
    // presses arrive as their own key events, and clearing on the Ctrl of
    // Ctrl+C would eat the selection before the C that copies it.
    const bool modifierKey = key == Qt::Key_Shift || key == Qt::Key_Control
        || key == Qt::Key_Alt || key == Qt::Key_Meta || key == Qt::Key_AltGr
        || key == Qt::Key_CapsLock || key == Qt::Key_NumLock;
    if (!modifierKey && hasSelection()) {
        clearSelection();
        update();
    }
    if ((control || meta) && key >= Qt::Key_A && key <= Qt::Key_Z) {
        const char byte = char(key - Qt::Key_A + 1);
        sendBytes(QByteArray(1, byte));
        event->accept();
        return;
    }
    if (meta || (control && key > Qt::Key_Z)) {
        // Other command/control combinations belong to the shell UI.
        event->ignore();
        return;
    }

    QByteArray sequence;
    switch (key) {
    case Qt::Key_Return:
    case Qt::Key_Enter:
        sequence = "\r";
        break;
    case Qt::Key_Backspace:
        sequence = "\x7f";
        break;
    case Qt::Key_Tab:
        sequence = "\t";
        break;
    case Qt::Key_Backtab:
        sequence = "\x1b[Z";
        break;
    case Qt::Key_Escape:
        sequence = "\x1b";
        break;
    case Qt::Key_Delete:
        sequence = "\x1b[3~";
        break;
    case Qt::Key_Insert:
        sequence = "\x1b[2~";
        break;
    case Qt::Key_Up:
        sequence = "\x1b[A";
        break;
    case Qt::Key_Down:
        sequence = "\x1b[B";
        break;
    case Qt::Key_Right:
        sequence = "\x1b[C";
        break;
    case Qt::Key_Left:
        sequence = "\x1b[D";
        break;
    case Qt::Key_Home:
        sequence = "\x1b[H";
        break;
    case Qt::Key_End:
        sequence = "\x1b[F";
        break;
    case Qt::Key_PageUp:
        sequence = "\x1b[5~";
        break;
    case Qt::Key_PageDown:
        sequence = "\x1b[6~";
        break;
    default:
        break;
    }
    if (!sequence.isEmpty()) {
        sendBytes(sequence);
        event->accept();
        return;
    }

    const QString text = event->text();
    if (!text.isEmpty() && !control) {
        if (alt)
            sendBytes(QByteArray(1, '\x1b') + text.toUtf8());
        else
            sendText(text);
        event->accept();
        return;
    }
    event->ignore();
}

void TerminalItem::inputMethodEvent(QInputMethodEvent* event)
{
    if (!event->commitString().isEmpty())
        sendText(event->commitString());
    m_preedit = event->preeditString();
    update();
    event->accept();
}

QVariant TerminalItem::inputMethodQuery(Qt::InputMethodQuery query) const
{
    switch (query) {
    case Qt::ImCursorRectangle:
        return QRectF(m_cursorCol * m_cellWidth, m_cursorRow * m_cellHeight, m_cellWidth,
                      m_cellHeight);
    case Qt::ImEnabled:
        return true;
    case Qt::ImHints:
        return int(Qt::ImhNoPredictiveText);
    default:
        return QQuickPaintedItem::inputMethodQuery(query);
    }
}

void TerminalItem::wheelEvent(QWheelEvent* event)
{
    const int steps = event->angleDelta().y() / 120;
    if (steps == 0 || !m_session) {
        event->ignore();
        return;
    }
    // The session decides: on the alternate screen (vim, less) the wheel
    // becomes arrow keys for the program; on the primary screen it walks
    // the scrollback.
    QMetaObject::invokeMethod(m_session.data(), "wheelScroll", Q_ARG(int, steps * 3));
    event->accept();
}

void TerminalItem::mousePressEvent(QMouseEvent* event)
{
    forceActiveFocus();
    if (event->button() != Qt::LeftButton || !m_hasFrame) {
        event->accept();
        return;
    }

    // A Ctrl+Click opens the path under the pointer instead of selecting.
    if (event->modifiers().testFlag(Qt::ControlModifier)) {
        openLinkAt(event->position());
        event->accept();
        return;
    }
    const QPointF position = event->position();
    if (scrollbarVisible()
        && scrollbarTrack().adjusted(-ScrollbarMargin, 0.0, ScrollbarMargin, 0.0)
               .contains(position)) {
        const QRectF thumb = scrollbarThumb();
        m_scrollbarDrag = true;
        m_scrollbarGrabOffset = thumb.contains(position) ? position.y() - thumb.top()
                                                         : thumb.height() / 2.0;
        updateScrollbarDrag(position);
        event->accept();
        return;
    }
    m_selecting = true;
    m_hasSelection = false;
    m_selAnchorRow = qBound(0, int(position.y() / m_cellHeight), m_rows - 1);
    m_selAnchorColumn = qBound(0, int(position.x() / m_cellWidth), m_cols - 1);
    m_selHeadRow = m_selAnchorRow;
    m_selHeadColumn = m_selAnchorColumn;
    update();
    event->accept();
}

/// Ctrl+Click opens the path or URL under the pointer. The match happens
/// on click, never per frame, so a pathological line costs nothing until
/// the user actually points at it. `path:line:col`, `path:line` and
/// `http(s)://` URLs are recognized; a relative path is resolved by the
/// QML side against the session's reported directory.
void TerminalItem::openLinkAt(const QPointF& position)
{
    const int row = qBound(0, int(position.y() / m_cellHeight), m_rows - 1);
    const int column = qBound(0, int(position.x() / m_cellWidth), m_cols - 1);

    QString text;
    text.reserve(m_cols);
    for (int x = 0; x < m_cols; ++x) {
        const TerminalCell& cell = cellAt(row, x);
        text.append(cell.ch == 0 ? QChar(u' ') : character(cell.ch));
    }

    static const QRegularExpression pattern(
        QStringLiteral("\\b(?:https?://[^\\s]+"
                       "|(?:[A-Za-z]:)?[\\w .~/\\\\-]*[\\w./\\\\-]+"
                       "(?:\\.[A-Za-z0-9]{1,5}[/\\\\]?)?)"
                       "(?::(\\d+)(?::(\\d+))?)?"),
        QRegularExpression::UseUnicodePropertiesOption);

    for (QRegularExpressionMatchIterator it = pattern.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        if (column < match.capturedStart() || column > match.capturedEnd())
            continue;
        const QString target = match.captured(0).trimmed();
        if (target.isEmpty())
            return;
        emit linkActivated(target, match.captured(1).toInt(), match.captured(2).toInt());
        return;
    }
}

void TerminalItem::mouseMoveEvent(QMouseEvent* event)
{
    if (m_scrollbarDrag) {
        updateScrollbarDrag(event->position());
        event->accept();
        return;
    }
    if (m_selecting) {
        extendSelection(qBound(0, int(event->position().y() / m_cellHeight), m_rows - 1),
                        qBound(0, int(event->position().x() / m_cellWidth), m_cols - 1));
        event->accept();
        return;
    }
    QQuickPaintedItem::mouseMoveEvent(event);
}

void TerminalItem::mouseReleaseEvent(QMouseEvent* event)
{
    if (m_scrollbarDrag) {
        m_scrollbarDrag = false;
        update();
        event->accept();
        return;
    }
    if (m_selecting) {
        m_selecting = false;
        // A click that never moved is not a selection; it is the gesture that
        // dismisses the previous one.
        if (m_selAnchorRow == m_selHeadRow && m_selAnchorColumn == m_selHeadColumn)
            clearSelection();
        // copyOnSelect: the gesture that made the selection is the one
        // that copies it, no Ctrl+C required.
        if (m_copyOnSelect && hasSelection())
            copySelection();
        update();
        event->accept();
        return;
    }
    QQuickPaintedItem::mouseReleaseEvent(event);
}

void TerminalItem::mouseDoubleClickEvent(QMouseEvent* event)
{
    if (event->button() == Qt::LeftButton && m_hasFrame) {
        selectWordAt(qBound(0, int(event->position().y() / m_cellHeight), m_rows - 1),
                     qBound(0, int(event->position().x() / m_cellWidth), m_cols - 1));
        event->accept();
        return;
    }
    QQuickPaintedItem::mouseDoubleClickEvent(event);
}

void TerminalItem::geometryChange(const QRectF& newGeometry, const QRectF& oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    measureCells();
    applySizeToSession();
    update();
}

void TerminalItem::focusInEvent(QFocusEvent* event)
{
    QQuickPaintedItem::focusInEvent(event);
    QGuiApplication::inputMethod()->update(Qt::ImQueryAll);
}

void TerminalItem::focusOutEvent(QFocusEvent* event)
{
    QQuickPaintedItem::focusOutEvent(event);
    m_preedit.clear();
    update();
}

void TerminalItem::sendBytes(const QByteArray& bytes)
{
    if (!m_session || bytes.isEmpty())
        return;
    QMetaObject::invokeMethod(m_session.data(), "writeBytes", Q_ARG(QByteArray, bytes));
}

void TerminalItem::sendText(const QString& text)
{
    if (!m_session || text.isEmpty())
        return;
    QMetaObject::invokeMethod(m_session.data(), "write", Q_ARG(QString, text));
}

void TerminalItem::paste()
{
    const QString text = QGuiApplication::clipboard()->text();
    if (text.isEmpty())
        return;
    // The session encodes the paste the way the running program asked for
    // it: bracketed when it enabled mode 2004, so a vim or tmux sees one
    // paste instead of a run of Enters; otherwise newlines become carriage
    // returns.
    QMetaObject::invokeMethod(m_session.data(), "pasteText", Q_ARG(QString, text));
}

void TerminalItem::selectionBounds(int& firstRow, int& firstColumn, int& lastRow,
                                   int& lastColumn) const
{
    if (m_selAnchorRow < m_selHeadRow
        || (m_selAnchorRow == m_selHeadRow && m_selAnchorColumn <= m_selHeadColumn)) {
        firstRow = m_selAnchorRow;
        firstColumn = m_selAnchorColumn;
        lastRow = m_selHeadRow;
        lastColumn = m_selHeadColumn;
    } else {
        firstRow = m_selHeadRow;
        firstColumn = m_selHeadColumn;
        lastRow = m_selAnchorRow;
        lastColumn = m_selAnchorColumn;
    }
}

void TerminalItem::clearSelection()
{
    m_hasSelection = false;
    m_selecting = false;
}

void TerminalItem::extendSelection(int row, int column)
{
    m_selHeadRow = row;
    m_selHeadColumn = column;
    m_hasSelection = true;
    update();
}

void TerminalItem::selectWordAt(int row, int column)
{
    int start = column;
    int end = column;
    if (wordCharacter(cellAt(row, column).ch)) {
        while (start > 0 && wordCharacter(cellAt(row, start - 1).ch))
            --start;
        while (end + 1 < m_cols && wordCharacter(cellAt(row, end + 1).ch))
            ++end;
    }
    m_selAnchorRow = row;
    m_selAnchorColumn = start;
    m_selHeadRow = row;
    m_selHeadColumn = end;
    m_hasSelection = true;
    update();
}

QString TerminalItem::selectedText() const
{
    if (!m_hasSelection)
        return QString();
    int firstRow, firstColumn, lastRow, lastColumn;
    selectionBounds(firstRow, firstColumn, lastRow, lastColumn);
    firstRow = qMax(0, firstRow);
    lastRow = qMin(lastRow, m_rows - 1);
    if (lastRow < firstRow)
        return QString();
    firstColumn = qBound(0, firstColumn, m_cols - 1);
    lastColumn = qBound(0, lastColumn, m_cols - 1);

    QStringList lines;
    for (int row = firstRow; row <= lastRow; ++row) {
        const int start = row == firstRow ? firstColumn : 0;
        const int end = row == lastRow ? lastColumn : m_cols - 1;
        QString line;
        for (int column = start; column <= end; ++column) {
            const TerminalCell& cell = cellAt(row, column);
            if (cell.flags & FlagSpacer)
                continue;
            line.append(character(cell.ch));
        }
        while (line.endsWith(QLatin1Char(' ')))
            line.chop(1);
        lines.append(line);
    }
    return lines.join(QLatin1Char('\n'));
}

void TerminalItem::copySelection()
{
    const QString text = selectedText();
    if (!text.isEmpty())
        QGuiApplication::clipboard()->setText(text);
}

bool TerminalItem::scrollbarVisible() const
{
    return m_historySize > 0 && m_rows > 0 && m_cols > 0;
}

QRectF TerminalItem::scrollbarTrack() const
{
    return QRectF(width() - ScrollbarWidth - ScrollbarMargin, ScrollbarMargin, ScrollbarWidth,
                  qMax(0.0, height() - 2.0 * ScrollbarMargin));
}

QRectF TerminalItem::scrollbarThumb() const
{
    const QRectF track = scrollbarTrack();
    if (m_historySize == 0 || m_rows <= 0 || track.height() <= 0.0)
        return QRectF();
    const qreal total = qreal(m_historySize) + qreal(m_rows);
    const qreal lower = qMin(ScrollbarMinThumb, track.height());
    const qreal height = qBound(lower, track.height() * qreal(m_rows) / total, track.height());
    const qreal maxTop = qMax(0.0, track.height() - height);
    const quint32 offset = qMin<quint32>(m_displayOffset, m_historySize);
    const qreal fraction = qreal(m_historySize - offset) / qreal(m_historySize);
    return QRectF(track.left(), track.top() + maxTop * fraction, track.width(), height);
}

void TerminalItem::scrollToOffset(int offset)
{
    if (!m_session)
        return;
    QMetaObject::invokeMethod(m_session.data(), "scrollTo", Q_ARG(int, offset));
}

void TerminalItem::updateScrollbarDrag(const QPointF& position)
{
    if (m_historySize == 0)
        return;
    const QRectF track = scrollbarTrack();
    const qreal height = scrollbarThumb().height();
    const qreal maxTop = qMax(0.0, track.height() - height);
    if (maxTop <= 0.0) {
        scrollToOffset(int(m_historySize));
        return;
    }
    const qreal top = qBound(track.top(), position.y() - m_scrollbarGrabOffset,
                             track.top() + maxTop);
    const qreal fraction = (top - track.top()) / maxTop;
    scrollToOffset(int(qRound((1.0 - fraction) * qreal(m_historySize))));
}

bool TerminalItem::privateUse(quint32 codepoint)
{
    return (codepoint >= 0xe000 && codepoint <= 0xf8ff)
        || (codepoint >= 0xf0000 && codepoint <= 0xffffd)
        || (codepoint >= 0x100000 && codepoint <= 0x10fffd);
}

QString TerminalItem::fallbackFamily(quint32 codepoint)
{
    if (!privateUse(codepoint))
        return QString();
    const auto cached = m_fallbackFamilies.constFind(codepoint);
    if (cached != m_fallbackFamilies.constEnd())
        return *cached;

    QString family;
    if (!m_primaryMetrics.inFontUcs4(codepoint)) {
        if (m_fallbackCandidates.isEmpty()) {
            const QStringList installed = QFontDatabase::families();
            QStringList nerd;
            QStringList rest;
            for (const QString& name : installed) {
                if (name.contains(QLatin1String("Nerd"), Qt::CaseInsensitive))
                    nerd.append(name);
                else
                    rest.append(name);
            }
            m_fallbackCandidates = nerd + rest;
        }
        const QStringList primary = m_font.families();
        for (const QString& name : m_fallbackCandidates) {
            if (primary.contains(name))
                continue;
            QFont candidate(m_font);
            candidate.setFamilies({name});
            if (QFontMetricsF(candidate).inFontUcs4(codepoint)) {
                family = name;
                break;
            }
        }
    }
    m_fallbackFamilies.insert(codepoint, family);
    return family;
}

QColor TerminalItem::colour(QRgb argb) const
{
    return QColor::fromRgba(argb);
}

QFont TerminalItem::cellFont(quint8 flags, const QString& family) const
{
    QFont font = styleFont(flags);
    if (!family.isEmpty())
        font.setFamilies({family});
    return font;
}

QFont TerminalItem::styleFont(quint8 flags) const
{
    QFont font = m_font;
    const bool bold = (flags & FlagBold) != 0;
    font.setBold(bold);
    font.setItalic((flags & FlagItalic) != 0);
    font.setUnderline((flags & FlagUnderline) != 0);
    font.setStrikeOut((flags & FlagStrikeout) != 0);
    // The bundled face is a variable font: `wght` is where its real Bold
    // instance lives, and QFont::setBold only decides which instance a
    // non-variable custom family synthesises from.
    font.setVariableAxis(QFont::Tag("wght"), bold ? 700.0f : 400.0f);
    return font;
}

const TerminalCell& TerminalItem::cellAt(int row, int column) const
{
    static const TerminalCell empty;
    if (row < 0 || row >= m_rows || column < 0 || column >= m_cols)
        return empty;
    return m_cells[row * m_cols + column];
}

} // namespace dae_qt
