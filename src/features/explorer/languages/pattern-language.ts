import type { HighlightTokenClass, TokenRange } from "@tanstack/highlight/core";

/** One ordered regex rule; earlier rules claim characters first. */
export type TokenPattern = {
  className: HighlightTokenClass;
  regex: RegExp;
  group?: number;
};

/**
 * Ordered, first-wins token collection: pre-scanned ranges (comments,
 * strings) always beat pattern matches, and earlier patterns beat later
 * ones. Mirrors the contract of the package's internal collector, which is
 * not part of its public exports.
 */
export function collectTokenRanges(
  code: string,
  patterns: readonly TokenPattern[],
  initial: readonly TokenRange[] = [],
): TokenRange[] {
  const ranges = [...initial];
  const occupied = new Uint8Array(code.length);
  for (const range of initial) occupied.fill(1, range.start, range.end);
  for (const pattern of patterns) {
    const regex = new RegExp(
      pattern.regex.source,
      pattern.regex.flags.includes("g") ? pattern.regex.flags : `${pattern.regex.flags}g`,
    );
    let match: RegExpExecArray | null;
    while ((match = regex.exec(code))) {
      if (match[0].length === 0) {
        regex.lastIndex++;
        continue;
      }
      const value = pattern.group === undefined ? match[0] : match[pattern.group];
      if (!value) continue;
      const offset = pattern.group === undefined ? 0 : match[0].indexOf(value);
      const start = match.index + offset;
      const end = start + value.length;
      if (start >= end || isOccupied(occupied, start, end)) continue;
      ranges.push({ start, end, className: pattern.className });
      occupied.fill(1, start, end);
    }
  }
  return ranges;
}

export function scanQuoted(code: string, start: number, escape = "\\", multiline = false): number {
  const quote = code[start];
  let index = start + 1;
  while (index < code.length) {
    if (escape && code[index] === escape) index += 2;
    else if (code[index] === quote) return index + 1;
    else if (!multiline && code[index] === "\n") return index;
    else index++;
  }
  return code.length;
}

export function scanLineComment(code: string, start: number): number {
  const end = code.indexOf("\n", start);
  return end < 0 ? code.length : end;
}

export function scanBlockComment(code: string, start: number, nestable = false): number {
  let depth = 1;
  let index = start + 2;
  while (index < code.length && depth > 0) {
    if (nestable && code[index] === "/" && code[index + 1] === "*") {
      depth++;
      index += 2;
    } else if (code[index] === "*" && code[index + 1] === "/") {
      depth--;
      index += 2;
    } else {
      index++;
    }
  }
  return index;
}

export function isWordChar(character: string | undefined): boolean {
  return character !== undefined && /[A-Za-z0-9_]/.test(character);
}

export function keywordPattern(keywords: readonly string[], flags = ""): TokenPattern {
  return {
    className: "keyword",
    regex: new RegExp(`\\b(?:${keywords.join("|")})\\b`, flags),
  };
}

export function literalPattern(literals: readonly string[], flags = ""): TokenPattern {
  return {
    className: "literal",
    regex: new RegExp(`\\b(?:${literals.join("|")})\\b`, flags),
  };
}

/** Integer and float literals with radix prefixes, digit separators, and an optional type suffix. */
export function numberPattern(suffix = ""): TokenPattern {
  return {
    className: "number",
    regex: new RegExp(
      `\\b(?:0x[\\da-f](?:_?[\\da-f])*|0b[01](?:_?[01])*|0o[0-7](?:_?[0-7])*|\\d(?:_?\\d)*(?:\\.\\d(?:_?\\d)*)?)(?:[eE][+-]?\\d+)?${suffix}\\b`,
      "i",
    ),
  };
}

export const pascalTypePattern: TokenPattern = {
  className: "type",
  regex: /\b[A-Z]\w*\b/g,
};

/** Call-site names; the leading `.` guard leaves `obj.method(` to the property rule. */
export const functionCallPattern: TokenPattern = {
  className: "function",
  regex: /(^|[^\w.])([A-Za-z_]\w*)(?=\s*\()/g,
  group: 2,
};

export const propertyPattern: TokenPattern = {
  className: "property",
  regex: /\.([A-Za-z_]\w*)/g,
  group: 1,
};

export const annotationPattern: TokenPattern = {
  className: "function",
  regex: /@[A-Za-z_]\w*/g,
};

function isOccupied(occupied: Uint8Array, start: number, end: number): boolean {
  for (let index = start; index < end; index++) {
    if (occupied[index]) return true;
  }
  return false;
}
