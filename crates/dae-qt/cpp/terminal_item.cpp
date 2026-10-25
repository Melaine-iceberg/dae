#include "terminal_item.h"

#include <QClipboard>
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

constexpr int FrameHeader = 4 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 4;
constexpr int FrameCell = 4 + 4 + 4 + 1;

quint16 readU16(const uchar* data)
{
    return quint16(data[0]) | (quint16(data[1]) << 8);
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

} // namespace

TerminalItem::TerminalItem(QQuickItem* parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    setAcceptedMouseButtons(Qt::LeftButton);
    setFlag(ItemAcceptsInputMethod, true);
    setFocus(true);
    setActiveFocusOnTab(true);

    m_font.setFamilies({ QStringLiteral("Cascadia Mono"), QStringLiteral("Consolas"),
                         QStringLiteral("Courier New") });
    m_font.setStyleHint(QFont::Monospace);
    m_font.setFixedPitch(true);
    m_font.setPixelSize(13);
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
    applySizeToSession();
    refresh();
}

void TerminalItem::setDark(bool dark)
{
    if (m_dark == dark)
        return;
    m_dark = dark;
    emit darkChanged();
    update();
}

void TerminalItem::setFontSize(int size)
{
    const int clamped = qBound(6, size, 32);
    if (m_font.pixelSize() == clamped)
        return;
    m_font.setPixelSize(clamped);
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
    if (trimmed.isEmpty()) {
        m_font.setFamilies({ QStringLiteral("Cascadia Mono"), QStringLiteral("Consolas"),
                             QStringLiteral("Courier New") });
    } else {
        m_font.setFamilies({ trimmed });
    }
    measureCells();
    applySizeToSession();
    emit fontChanged();
    update();
}

void TerminalItem::measureCells()
{
    const QFontMetricsF metrics(m_font);
    m_cellWidth = qMax(1.0, metrics.horizontalAdvance(QLatin1Char('M')));
    m_cellHeight = qMax(1.0, metrics.lineSpacing() * m_lineHeight);
    m_ascent = metrics.ascent();
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
    if (m_session) {
        QMetaObject::invokeMethod(m_session.data(), "resize", Q_ARG(int, cols), Q_ARG(int, rows));
    }
}

void TerminalItem::refresh()
{
    if (!m_session) {
        m_hasFrame = false;
        m_cells.clear();
        m_cols = 0;
        m_rows = 0;
        update();
        return;
    }
    QByteArray frame;
    if (QMetaObject::invokeMethod(m_session.data(), "snapshot",
                                  Q_RETURN_ARG(QByteArray, frame))) {
        decodeFrame(frame);
    }
    update();
}

void TerminalItem::decodeFrame(const QByteArray& frame)
{
    if (frame.size() < FrameHeader)
        return;
    const auto* data = reinterpret_cast<const uchar*>(frame.constData());
    if (std::memcmp(data, "DAET", 4) != 0 || data[4] != 1)
        return;

    const int cols = readU16(data + 5);
    const int rows = readU16(data + 7);
    if (cols <= 0 || rows <= 0 || cols > 4096 || rows > 4096)
        return;
    if (frame.size() < FrameHeader + cols * rows * FrameCell)
        return;

    m_cols = cols;
    m_rows = rows;
    m_cursorCol = readU16(data + 9);
    m_cursorRow = readU16(data + 11);
    m_cursorVisible = data[13] != 0;
    m_cursorShape = data[14];
    m_displayOffset = readU32(data + 15);

    m_cells.resize(cols * rows);
    const uchar* cell = data + FrameHeader;
    for (int index = 0; index < cols * rows; ++index) {
        TerminalCell& target = m_cells[index];
        target.ch = readU32(cell);
        target.fg = readU32(cell + 4);
        target.bg = readU32(cell + 8);
        target.flags = cell[12];
        cell += FrameCell;
    }
    m_hasFrame = true;
}

void TerminalItem::paint(QPainter* painter)
{
    painter->fillRect(boundingRect(), QColor(0x14, 0x16, 0x1a));

    if (!m_hasFrame || m_cols <= 0 || m_rows <= 0) {
        painter->setPen(QColor(0x65, 0x70, 0x80));
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

    // Then text, one drawText per style/colour run.
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
            if (first.flags & FlagWide) {
                painter->setFont(styleFont(first.flags));
                painter->setPen(colour(first.fg));
                painter->drawText(QRectF(column * m_cellWidth, y, m_cellWidth * 2.0, m_cellHeight),
                                  Qt::TextSingleLine | Qt::AlignLeft | Qt::AlignVCenter,
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
                text.append(character(cell.ch));
                ++column;
            }
            if (text.isEmpty()) {
                ++column;
                continue;
            }
            painter->setFont(styleFont(style));
            painter->setPen(colour(foreground));
            painter->drawText(QRectF(start * m_cellWidth, y, (column - start) * m_cellWidth,
                                     m_cellHeight),
                              Qt::TextSingleLine | Qt::AlignLeft | Qt::AlignVCenter, text);
        }
    }

    if (!m_preedit.isEmpty()) {
        const QRectF at(m_cursorCol * m_cellWidth, m_cursorRow * m_cellHeight,
                        m_cellWidth * qMax(1, m_preedit.size()), m_cellHeight);
        painter->setPen(colour(0xffd8dee9));
        painter->drawText(at, Qt::TextSingleLine | Qt::AlignLeft | Qt::AlignVCenter, m_preedit);
        painter->drawLine(at.bottomLeft(), at.bottomRight());
    }

    const bool drawCursor = m_cursorVisible && (!hasActiveFocus() || m_blinkOn);
    if (drawCursor) {
        const QRectF cursor(m_cursorCol * m_cellWidth, m_cursorRow * m_cellHeight, m_cellWidth,
                            m_cellHeight);
        const QColor accent(0x6e, 0xa8, 0xfe);
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

    if (((control || meta) && key == Qt::Key_V) || (control && shift && key == Qt::Key_V)) {
        paste();
        event->accept();
        return;
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
    QMetaObject::invokeMethod(m_session.data(), "scroll", Q_ARG(int, steps * 3));
    event->accept();
}

void TerminalItem::mousePressEvent(QMouseEvent* event)
{
    forceActiveFocus();
    event->accept();
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
    // A terminal treats a newline as "run", not "open a line": pasted line
    // breaks must become carriage returns or a multi-line paste stalls in the
    // shell's line editor.
    QString normalized = text;
    normalized.replace(QLatin1String("\r\n"), QLatin1String("\r"));
    normalized.replace(QLatin1Char('\n'), QLatin1Char('\r'));
    sendText(normalized);
}

QColor TerminalItem::colour(QRgb argb) const
{
    return QColor::fromRgba(argb);
}

QFont TerminalItem::styleFont(quint8 flags) const
{
    QFont font = m_font;
    font.setBold((flags & FlagBold) != 0);
    font.setItalic((flags & FlagItalic) != 0);
    font.setUnderline((flags & FlagUnderline) != 0);
    font.setStrikeOut((flags & FlagStrikeout) != 0);
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
