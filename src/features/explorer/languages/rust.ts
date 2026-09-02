import { defineLanguage, type TokenRange } from "@tanstack/highlight/core";
import {
  collectTokenRanges,
  functionCallPattern,
  isWordChar,
  keywordPattern,
  literalPattern,
  numberPattern,
  pascalTypePattern,
  propertyPattern,
  scanBlockComment,
  scanLineComment,
  scanQuoted,
  type TokenPattern,
} from "./pattern-language";

const KEYWORDS = [
  "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
  "default", "do", "dyn", "else", "enum", "extern", "final", "fn", "for", "if",
  "impl", "in", "let", "loop", "match", "mod", "move", "mut", "override", "priv",
  "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
  "try", "type", "typeof", "union", "unsafe", "unsized", "use", "virtual",
  "where", "while", "yield",
];

const PATTERNS: readonly TokenPattern[] = [
  numberPattern("(?:[iuf](?:8|16|32|64|128|size))?"),
  keywordPattern(KEYWORDS),
  literalPattern(["true", "false"]),
  { className: "type", regex: /\b(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64|bool|char|str)\b/g },
  { className: "meta", regex: /#!?\[/g },
  { className: "function", regex: /\b[A-Za-z_]\w*!(?!=)/g },
  functionCallPattern,
  pascalTypePattern,
  propertyPattern,
  { className: "operator", regex: /->|=>|\?/g },
];

/**
 * Rust grammar for TanStack Highlight, which ships no Rust tokenizer.
 * Comments, raw strings, char literals and lifetimes need a character-level
 * first pass because they cannot be told apart with plain regex alternation.
 */
export const rust = defineLanguage({
  name: "rust",
  aliases: ["rs"],
  tokenize: (code) => collectTokenRanges(code, PATTERNS, scanInitial(code)),
});

function scanInitial(code: string): TokenRange[] {
  const ranges: TokenRange[] = [];
  let index = 0;
  while (index < code.length) {
    const character = code[index];
    const next = code[index + 1];

    if (character === "/" && next === "/") {
      const end = scanLineComment(code, index + 2);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === "/" && next === "*") {
      // Rust block comments nest.
      const end = scanBlockComment(code, index, true);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }

    // Raw strings `r#"…"#` and byte variants `br#"…"#` (any number of hashes).
    if ((character === "r" || character === "b") && !isWordChar(code[index - 1])) {
      const rPos = character === "r" ? index : next === "r" ? index + 1 : -1;
      if (rPos >= 0) {
        let cursor = rPos + 1;
        while (code[cursor] === "#") cursor++;
        if (code[cursor] === '"') {
          const closing = `"${"#".repeat(cursor - rPos - 1)}`;
          const closeAt = code.indexOf(closing, cursor + 1);
          const end = closeAt < 0 ? code.length : closeAt + closing.length;
          ranges.push({ start: index, end, className: "string" });
          index = end;
          continue;
        }
      }
      if (character === "b" && next === '"') {
        const end = scanQuoted(code, index + 1);
        ranges.push({ start: index, end, className: "string" });
        index = end;
        continue;
      }
    }

    if (character === '"') {
      const end = scanQuoted(code, index);
      ranges.push({ start: index, end, className: "string" });
      index = end;
      continue;
    }

    if (character === "'" || (character === "b" && next === "'" && !isWordChar(code[index - 1]))) {
      const quote = character === "'" ? index : index + 1;
      const charEnd = scanCharLiteral(code, quote);
      if (charEnd > 0) {
        ranges.push({ start: index, end: charEnd, className: "literal" });
        index = charEnd;
        continue;
      }
      const lifetimeEnd = scanLifetime(code, quote);
      if (lifetimeEnd > 0) {
        ranges.push({ start: quote, end: lifetimeEnd, className: "type" });
        index = lifetimeEnd;
        continue;
      }
    }

    index++;
  }
  return ranges;
}

/** `'a'` / `'\n'` / `'\u{1F600}'` / `b'x'`; returns the end offset or -1. */
function scanCharLiteral(code: string, quote: number): number {
  let cursor = quote + 1;
  if (code[cursor] === "\\") {
    cursor++;
    if (code[cursor] === "u" && code[cursor + 1] === "{") {
      const close = code.indexOf("}", cursor + 2);
      if (close < 0) return -1;
      cursor = close + 1;
    } else if (code[cursor] === "x") {
      cursor += 3;
    } else {
      cursor++;
    }
  } else if (code[cursor] !== "'" && code[cursor] !== undefined) {
    cursor++;
  } else {
    return -1;
  }
  return code[cursor] === "'" ? cursor + 1 : -1;
}

/** `'a` / `'static` / `'_`; returns the end offset or -1. */
function scanLifetime(code: string, quote: number): number {
  const cursor = quote + 1;
  if (!/[A-Za-z_]/.test(code[cursor] ?? "")) return -1;
  let end = cursor;
  while (end < code.length && /[A-Za-z0-9_]/.test(code[end])) end++;
  return end;
}
