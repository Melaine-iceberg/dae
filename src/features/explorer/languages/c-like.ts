import { defineLanguage, type LanguageDefinition, type TokenRange } from "@tanstack/highlight/core";
import {
  annotationPattern,
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

type CLikeOptions = {
  keywords: readonly string[];
  literals?: readonly string[];
  types?: readonly string[];
  preprocessor?: boolean;
  annotations?: boolean;
  backtickStrings?: boolean;
  nestedBlockComments?: boolean;
  caseInsensitive?: boolean;
  /** Declaration keywords whose declared name should stay a type, not a call (Kotlin's `class Foo(...)`). */
  typeDeclKeywords?: readonly string[];
};

/**
 * C-family grammars for TanStack Highlight: comments and quoted literals
 * first, then keyword/type/call patterns shared across the family. Language
 * sets differ only in keywords, literals, and a few scanner switches.
 */
function createCLike(
  name: string,
  aliases: readonly string[],
  options: CLikeOptions,
): LanguageDefinition {
  return defineLanguage({
    name,
    aliases,
    tokenize: (code) => collectTokenRanges(code, patterns(options), scanInitial(code, options)),
  });
}

function patterns(options: CLikeOptions): readonly TokenPattern[] {
  const flags = options.caseInsensitive ? "i" : "";
  return [
    numberPattern("(?:[uUlLfF]{1,3})?"),
    keywordPattern(options.keywords, flags),
    literalPattern(options.literals ?? ["true", "false", "null", "nullptr", "nil"], flags),
    ...(options.preprocessor
      ? [{ className: "meta" as const, regex: /^[ \t]*#[ \t]*\w+/gm }]
      : []),
    ...(options.annotations ? [annotationPattern] : []),
    ...(options.typeDeclKeywords
      ? [
          {
            className: "type" as const,
            regex: new RegExp(`\\b(?:${options.typeDeclKeywords.join("|")})\\s+([A-Za-z_]\\w*)`, flags),
            group: 1,
          },
        ]
      : []),
    functionCallPattern,
    pascalTypePattern,
    ...(options.types
      ? [
          {
            className: "type" as const,
            regex: new RegExp(`\\b(?:${options.types.join("|")})\\b`, flags),
          },
        ]
      : []),
    propertyPattern,
  ];
}

function scanInitial(code: string, options: CLikeOptions): TokenRange[] {
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
      const end = scanBlockComment(code, index, options.nestedBlockComments);
      ranges.push({ start: index, end, className: "comment" });
      index = end;
      continue;
    }
    if (options.backtickStrings && character === "`") {
      const close = code.indexOf("`", index + 1);
      const end = close < 0 ? code.length : close + 1;
      ranges.push({ start: index, end, className: "string" });
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

const C_KEYWORDS = [
  "auto", "break", "case", "char", "const", "continue", "default", "do",
  "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline",
  "int", "long", "register", "restrict", "return", "short", "signed",
  "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned",
  "void", "volatile", "while", "bool", "_Bool", "_Static_assert",
];

const CPP_EXTRA = [
  "alignas", "alignof", "catch", "char8_t", "char16_t", "char32_t", "class",
  "concept", "consteval", "constexpr", "constinit", "const_cast",
  "co_await", "co_return", "co_yield", "decltype", "delete", "dynamic_cast",
  "explicit", "export", "friend", "mutable", "namespace", "new", "noexcept",
  "operator", "override", "private", "protected", "public",
  "reinterpret_cast", "requires", "static_assert", "static_cast", "template",
  "this", "thread_local", "throw", "try", "typeid", "typename", "using",
  "virtual", "final", "import", "module",
];

export const c = createCLike("c", [], {
  keywords: C_KEYWORDS,
  literals: ["true", "false", "NULL"],
  preprocessor: true,
});

export const cpp = createCLike("cpp", ["c++"], {
  keywords: [...C_KEYWORDS, ...CPP_EXTRA],
  literals: ["true", "false", "nullptr", "NULL"],
  preprocessor: true,
});

export const csharp = createCLike("csharp", ["cs", "c#"], {
  keywords: [
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch",
    "char", "checked", "class", "const", "continue", "decimal", "default",
    "delegate", "do", "double", "else", "enum", "event", "explicit", "extern",
    "finally", "fixed", "float", "for", "foreach", "get", "goto", "if",
    "implicit", "in", "init", "int", "interface", "internal", "is", "lock",
    "long", "namespace", "new", "object", "operator", "or", "out", "override",
    "params", "partial", "private", "protected", "public", "readonly",
    "record", "ref", "return", "sbyte", "sealed", "set", "short", "sizeof",
    "stackalloc", "static", "string", "struct", "switch", "this", "throw",
    "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using",
    "var", "virtual", "void", "volatile", "when", "where", "while", "with",
    "yield", "async", "await", "global", "file", "required", "scoped",
    "alias", "dynamic", "unmanaged", "nint", "nuint",
  ],
  preprocessor: true,
});

export const java = createCLike("java", [], {
  keywords: [
    "abstract", "assert", "boolean", "break", "byte", "case", "catch",
    "char", "class", "const", "continue", "default", "do", "double", "else",
    "enum", "extends", "final", "finally", "float", "for", "goto", "if",
    "implements", "import", "instanceof", "int", "interface", "long",
    "native", "new", "package", "private", "protected", "public", "return",
    "short", "static", "strictfp", "super", "switch", "synchronized", "this",
    "throw", "throws", "transient", "try", "void", "volatile", "while",
    "var", "record", "sealed", "yield", "permits",
  ],
  annotations: true,
});

export const kotlin = createCLike("kotlin", ["kt", "kts"], {
  keywords: [
    "as", "abstract", "actual", "annotation", "break", "by", "catch", "class",
    "companion", "const", "constructor", "continue", "crossinline", "data",
    "do", "dynamic", "else", "enum", "expect", "external", "final",
    "finally", "for", "fun", "get", "if", "import", "in", "infix", "init",
    "inline", "inner", "interface", "internal", "is", "lateinit", "noinline",
    "object", "open", "operator", "out", "override", "package", "param",
    "private", "property", "protected", "public", "reified", "return",
    "sealed", "set", "super", "suspend", "tailrec", "this", "throw", "try",
    "typealias", "val", "var", "vararg", "when", "where", "while",
  ],
  annotations: true,
  nestedBlockComments: true,
  typeDeclKeywords: ["class", "object", "interface"],
});

export const go = createCLike("go", ["golang"], {
  keywords: [
    "break", "case", "chan", "const", "continue", "default", "defer", "else",
    "fallthrough", "for", "func", "go", "goto", "if", "import", "interface",
    "map", "package", "range", "return", "select", "struct", "switch", "type",
    "var",
  ],
  literals: ["nil", "true", "false", "iota"],
  types: [
    "any", "bool", "byte", "complex64", "complex128", "error", "float32",
    "float64", "int", "int8", "int16", "int32", "int64", "rune", "string",
    "uint", "uint8", "uint16", "uint32", "uint64", "uintptr",
  ],
  backtickStrings: true,
});
