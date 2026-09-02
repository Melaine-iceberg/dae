import { defineLanguage, type TokenRange } from "@tanstack/highlight/core";
import {
  collectTokenRanges,
  functionCallPattern,
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

/**
 * Ruby, Lua, PHP and PowerShell grammars for TanStack Highlight. Each pairs
 * a lexical first pass (comments and string forms specific to the language)
 * with ordered semantic patterns.
 */
export const ruby = defineLanguage({
  name: "ruby",
  aliases: ["rb"],
  tokenize: (code) => collectTokenRanges(code, RUBY_PATTERNS, scanHashComments(code, "\\")),
});

const RUBY_PATTERNS: readonly TokenPattern[] = [
  numberPattern(),
  keywordPattern([
    "alias", "and", "begin", "break", "case", "class", "def", "do", "else",
    "elsif", "end", "ensure", "for", "if", "in", "module", "next", "not",
    "or", "redo", "rescue", "retry", "return", "self", "super", "then",
    "undef", "unless", "until", "when", "while", "yield", "require",
    "require_relative", "attr_accessor", "attr_reader", "attr_writer",
    "include", "extend",
  ]),
  literalPattern(["nil", "true", "false"]),
  { className: "literal", regex: /:[A-Za-z_]\w*[?!=]?/g },
  { className: "function", regex: /\bdef\s+(?:self\.)?([A-Za-z_]\w*[?!=]?)/g, group: 1 },
  { className: "variable", regex: /@@?[A-Za-z_]\w*|\$[A-Za-z_]\w*/g },
  functionCallPattern,
  pascalTypePattern,
  propertyPattern,
];

export const lua = defineLanguage({
  name: "lua",
  aliases: [],
  tokenize: (code) => collectTokenRanges(code, LUA_PATTERNS, scanLuaInitial(code)),
});

const LUA_PATTERNS: readonly TokenPattern[] = [
  numberPattern(),
  keywordPattern([
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto",
    "if", "in", "local", "not", "or", "repeat", "return", "then", "until",
    "while",
  ]),
  literalPattern(["nil", "true", "false"]),
  { className: "function", regex: /\bfunction\s+([A-Za-z_][\w.]*)/g, group: 1 },
  functionCallPattern,
  pascalTypePattern,
  propertyPattern,
];

export const php = defineLanguage({
  name: "php",
  aliases: [],
  tokenize: (code) => collectTokenRanges(code, PHP_PATTERNS, scanPhpInitial(code)),
});

const PHP_PATTERNS: readonly TokenPattern[] = [
  numberPattern(),
  keywordPattern(
    [
      "abstract", "and", "array", "as", "break", "callable", "case", "catch",
      "class", "clone", "const", "continue", "declare", "default", "do",
      "echo", "else", "elseif", "empty", "enum", "extends", "final",
      "finally", "fn", "for", "foreach", "function", "global", "goto", "if",
      "implements", "include", "include_once", "instanceof", "insteadof",
      "interface", "isset", "list", "match", "namespace", "new", "or",
      "print", "private", "protected", "public", "readonly", "require",
      "require_once", "return", "static", "switch", "throw", "trait", "try",
      "unset", "use", "var", "while", "xor", "yield",
    ],
    "i",
  ),
  literalPattern(["true", "false", "null"], "i"),
  {
    className: "type",
    regex: /\b(?:string|int|float|bool|iterable|object|mixed|void|self|parent|static)\b/i,
  },
  { className: "meta", regex: /<\?(?:php|=)?|\?>/g },
  { className: "variable", regex: /\$[A-Za-z_]\w*/g },
  functionCallPattern,
  pascalTypePattern,
];

export const powershell = defineLanguage({
  name: "powershell",
  aliases: ["ps1", "pwsh"],
  tokenize: (code) => collectTokenRanges(code, POWERSHELL_PATTERNS, scanPowerShellInitial(code)),
});

const POWERSHELL_PATTERNS: readonly TokenPattern[] = [
  numberPattern(),
  // Ahead of the literal rule so automatic variables keep their sigil: $true.
  { className: "variable", regex: /\$[A-Za-z_]\w*|\$\{[^}\n]*\}/g },
  keywordPattern(
    [
      "begin", "break", "catch", "class", "continue", "data", "define", "do",
      "dynamicparam", "else", "elseif", "end", "enum", "exit", "filter",
      "finally", "for", "foreach", "from", "function", "if", "in", "param",
      "process", "return", "switch", "throw", "trap", "try", "until", "using",
      "var", "while",
    ],
    "i",
  ),
  literalPattern(["true", "false", "null"], "i"),
  { className: "function", regex: /\bfunction\s+([A-Za-z_][\w-]*)/g, group: 1 },
  // Verb-Noun cmdlets; the noun may itself be multi-word PascalCase (Get-ChildItem).
  { className: "command", regex: /\b[A-Z][a-z]+-[A-Za-z][A-Za-z0-9]*\b/g },
  functionCallPattern,
];

/** `#` line comments plus single/double-quoted strings with a shared escape character. */
function scanHashComments(code: string, escape: string): TokenRange[] {
  const ranges: TokenRange[] = [];
  let index = 0;
  while (index < code.length) {
    const character = code[index];
    if (character === "#") {
      const end = scanLineComment(code, index + 1);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === '"' || character === "'") {
      const end = scanQuoted(code, index, escape);
      ranges.push({ start: index, end, className: "string" });
      index = end;
      continue;
    }
    index++;
  }
  return ranges;
}

/** PHP: `//`, `#` and block comments plus quoted strings. */
function scanPhpInitial(code: string): TokenRange[] {
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
    if (character === "#") {
      const end = scanLineComment(code, index + 1);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === "/" && next === "*") {
      const end = scanBlockComment(code, index);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === '"' || character === "'") {
      const end = scanQuoted(code, index);
      ranges.push({ start: index, end, className: "string" });
      index = end;
      continue;
    }
    index++;
  }
  return ranges;
}

/** PowerShell: `#` and `<# … #>` comments; `"…"` spans lines with backtick escapes, `'…'` is literal. */
function scanPowerShellInitial(code: string): TokenRange[] {
  const ranges: TokenRange[] = [];
  let index = 0;
  while (index < code.length) {
    const character = code[index];
    const next = code[index + 1];
    if (character === "#") {
      const end = scanLineComment(code, index + 1);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === "<" && next === "#") {
      const close = code.indexOf("#>", index + 2);
      const end = close < 0 ? code.length : close + 2;
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === '"' || character === "'") {
      const end = scanQuoted(code, index, character === '"' ? "`" : "", character === '"');
      ranges.push({ start: index, end, className: "string" });
      index = end;
      continue;
    }
    index++;
  }
  return ranges;
}

/** Lua: `--` comments (plain or long-bracket), long-bracket strings, and quoted strings. */
function scanLuaInitial(code: string): TokenRange[] {
  const ranges: TokenRange[] = [];
  let index = 0;
  while (index < code.length) {
    const character = code[index];
    const next = code[index + 1];
    if (character === "-" && next === "-") {
      const longEnd = scanLongBracket(code, index + 2);
      if (longEnd > 0) {
        ranges.push({ start: index, end: longEnd, className: "comment" });
        index = longEnd;
        continue;
      }
      const end = scanLineComment(code, index + 2);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (character === "[") {
      const end = scanLongBracket(code, index + 1);
      if (end > 0) {
        ranges.push({ start: index, end, className: "string" });
        index = end;
        continue;
      }
    }
    if (character === '"' || character === "'") {
      const end = scanQuoted(code, index);
      ranges.push({ start: index, end, className: "string" });
      index = end;
      continue;
    }
    index++;
  }
  return ranges;
}

/** `[=*[ … ]=*]`; returns the end offset when a long bracket starts at `start`, else -1. */
function scanLongBracket(code: string, start: number): number {
  let level = 0;
  let cursor = start;
  while (code[cursor] === "=") {
    level++;
    cursor++;
  }
  if (code[cursor] !== "[") return -1;
  const closing = `]${"=".repeat(level)}]`;
  const closeAt = code.indexOf(closing, cursor + 1);
  return closeAt < 0 ? code.length : closeAt + closing.length;
}
