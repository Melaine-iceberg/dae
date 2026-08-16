import {
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import { CircleNotchIcon, FolderOpenIcon, WarningIcon } from "@phosphor-icons/react";
import { openPath } from "@tauri-apps/plugin-opener";

import {
  commands,
  type ContentSearchFile,
  type ContentSearchMatch,
  type ContentSearchResponse,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

import { getFilePresentation } from "./file-icons";

/** Content scans read every candidate file, so they debounce longer than name search. */
const CONTENT_SEARCH_DEBOUNCE_MS = 350;
const COLLAPSED_MATCH_ROWS = 3;

export interface ContentSearchController {
  caseSensitive: boolean;
  error: string | null;
  isActive: boolean;
  isRegex: boolean;
  isSearching: boolean;
  query: string;
  response: ContentSearchResponse | null;
  setCaseSensitive: Dispatch<SetStateAction<boolean>>;
  setQuery: Dispatch<SetStateAction<string>>;
  setIsRegex: Dispatch<SetStateAction<boolean>>;
  setTypeFilter: Dispatch<SetStateAction<string>>;
  typeFilter: string;
}

export function useContentSearch(
  directoryPath: string | null,
  refreshToken: object | null,
  enabled: boolean,
): ContentSearchController {
  const requestVersionRef = useRef(0);
  const [query, setQuery] = useState("");
  const [isRegex, setIsRegex] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [typeFilter, setTypeFilter] = useState("");
  const [response, setResponse] = useState<ContentSearchResponse | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmedQuery = query.trim();
  const trimmedFilter = typeFilter.trim();

  useEffect(() => {
    requestVersionRef.current += 1;
    setQuery("");
    setResponse(null);
    setError(null);
    setIsSearching(false);
    void commands.cancelSearch().catch(() => undefined);
  }, [directoryPath]);

  useEffect(
    () => () => {
      requestVersionRef.current += 1;
      void commands.cancelSearch().catch(() => undefined);
    },
    [],
  );

  useEffect(() => {
    const requestVersion = ++requestVersionRef.current;
    setResponse(null);
    setError(null);

    if (!enabled || !directoryPath || !trimmedQuery) {
      setIsSearching(false);
      if (!trimmedQuery) {
        void commands.cancelSearch().catch(() => undefined);
      }
      return;
    }

    setIsSearching(true);
    const timeout = window.setTimeout(() => {
      void commands
        .searchFileContents(
          directoryPath,
          trimmedQuery,
          isRegex,
          caseSensitive,
          trimmedFilter || null,
        )
        .then((nextResponse) => {
          if (requestVersion === requestVersionRef.current) {
            setResponse(nextResponse);
          }
        })
        .catch((searchError: unknown) => {
          if (requestVersion === requestVersionRef.current) {
            setError(getErrorMessage(searchError));
          }
        })
        .finally(() => {
          if (requestVersion === requestVersionRef.current) {
            setIsSearching(false);
          }
        });
    }, CONTENT_SEARCH_DEBOUNCE_MS);

    return () => window.clearTimeout(timeout);
  }, [caseSensitive, directoryPath, enabled, isRegex, refreshToken, trimmedFilter, trimmedQuery]);

  return {
    caseSensitive,
    error,
    isActive: trimmedQuery.length > 0,
    isRegex,
    isSearching,
    query,
    response,
    setCaseSensitive,
    setQuery,
    setIsRegex,
    setTypeFilter,
    typeFilter,
  };
}

/** Regex / case / file-type controls shown while content search is active. */
export function ContentSearchToolbar({ search }: { search: ContentSearchController }) {
  const fileCount = search.response?.files.length ?? 0;
  const matchCount =
    search.response?.files.reduce((total, file) => total + file.matches.length, 0) ?? 0;

  return (
    <div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
      <div className="flex items-center gap-0.5">
        <Button
          aria-label="切换正则表达式"
          aria-pressed={search.isRegex}
          onClick={() => search.setIsRegex((value) => !value)}
          size="xs"
          title="正则表达式"
          type="button"
          variant={search.isRegex ? "secondary" : "ghost"}
        >
          .*
        </Button>
        <Button
          aria-label="切换区分大小写"
          aria-pressed={search.caseSensitive}
          onClick={() => search.setCaseSensitive((value) => !value)}
          size="xs"
          title="区分大小写"
          type="button"
          variant={search.caseSensitive ? "secondary" : "ghost"}
        >
          Aa
        </Button>
      </div>
      <Input
        aria-label="文件类型过滤"
        className="h-7 w-52"
        onChange={(event) => search.setTypeFilter(event.target.value)}
        placeholder="类型过滤，如 *.ts, rs, *.md"
        spellCheck={false}
        title="只搜索匹配的文件类型，支持扩展名或 glob，逗号分隔"
        value={search.typeFilter}
      />
      <span className="select-none">默认忽略 .git / node_modules / target</span>
      {search.isSearching && <CircleNotchIcon className="animate-spin" size={14} />}
      {!search.isSearching && search.response && (
        <span className="select-none tabular-nums">
          {fileCount.toLocaleString("zh-CN")} 个文件 · {matchCount.toLocaleString("zh-CN")} 处匹配
          {search.response.truncated ? "（已达上限，已截断）" : ""}
        </span>
      )}
    </div>
  );
}

/** Match list grouped by file; each row opens the file with the system default. */
export function ContentSearchResults({
  error,
  isSearching,
  onOpenLocation,
  query,
  response,
}: {
  error: string | null;
  isSearching: boolean;
  onOpenLocation: (directory: string) => void;
  query: string;
  response: ContentSearchResponse | null;
}) {
  if (error) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center select-none">
        <WarningIcon className="size-5 text-destructive" />
        <p className="text-[13px] text-destructive">{error}</p>
      </div>
    );
  }

  if (!isSearching && response && response.files.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center select-none">
        <p className="text-[13px] text-muted-foreground">
          未找到内容包含“{query}”的文件
        </p>
      </div>
    );
  }

  if (!response) {
    return <div className="min-h-0 flex-1" />;
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto px-3 py-2">
      <div className="mx-auto flex max-w-4xl flex-col gap-1.5">
        {response.files.map((file) => (
          <FileMatchGroup file={file} key={file.path} onOpenLocation={onOpenLocation} />
        ))}
      </div>
    </div>
  );
}

function FileMatchGroup({
  file,
  onOpenLocation,
}: {
  file: ContentSearchFile;
  onOpenLocation: (directory: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const fileName = fileNameOf(file.path);
  const presentation = getFilePresentation(fileName);
  const FileIcon = presentation.icon;
  const visibleMatches = expanded ? file.matches : file.matches.slice(0, COLLAPSED_MATCH_ROWS);
  const parentDirectory = parentDirectoryOf(file.path);

  return (
    <div className="overflow-hidden rounded-lg border border-foreground/5">
      <div className="flex items-center gap-2 bg-accent/40 px-3 py-1.5">
        <FileIcon className="size-4 shrink-0 text-muted-foreground" />
        <button
          className="min-w-0 flex-1 truncate text-left text-[13px] font-medium"
          onClick={() => setExpanded((value) => !value)}
          title={file.path}
          type="button"
        >
          {file.relativePath || fileName}
          <span className="ml-2 text-xs font-normal text-muted-foreground">
            {file.matches.length} 处匹配
          </span>
        </button>
        <Button
          aria-label="打开所在文件夹"
          onClick={() => parentDirectory && onOpenLocation(parentDirectory)}
          size="icon-xs"
          title="打开所在文件夹"
          type="button"
          variant="ghost"
        >
          <FolderOpenIcon />
        </Button>
      </div>
      <div className="flex flex-col">
        {visibleMatches.map((match) => (
          <button
            className="flex items-start gap-3 px-3 py-1 text-left text-[13px] transition-colors hover:bg-accent/40"
            key={match.lineNumber}
            onClick={() => void openPath(file.path)}
            title={`${file.path}:${match.lineNumber}`}
            type="button"
          >
            <span className="w-12 shrink-0 pt-px text-right text-xs text-muted-foreground tabular-nums select-none">
              {match.lineNumber}
            </span>
            <code className="min-w-0 flex-1 leading-relaxed break-all whitespace-pre-wrap">
              <HighlightedLine line={match} />
            </code>
          </button>
        ))}
        {file.matches.length > COLLAPSED_MATCH_ROWS && (
          <button
            className="px-3 py-1 text-left text-xs text-muted-foreground transition-colors hover:text-foreground"
            onClick={() => setExpanded((value) => !value)}
            type="button"
          >
            {expanded ? "收起匹配行" : `显示全部 ${file.matches.length} 处匹配`}
          </button>
        )}
      </div>
    </div>
  );
}

/** Splits a match row on its backend-provided character ranges and highlights them. */
function HighlightedLine({ line }: { line: ContentSearchMatch }) {
  if (line.ranges.length === 0) {
    return <>{line.lineText}</>;
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [start, end] of line.ranges) {
    const safeStart = Math.max(cursor, start);
    if (safeStart > cursor) {
      nodes.push(line.lineText.slice(cursor, safeStart));
    }
    if (end > safeStart) {
      nodes.push(
        <mark className="rounded-sm bg-primary/25 px-0.5 text-foreground" key={safeStart}>
          {line.lineText.slice(safeStart, end)}
        </mark>,
      );
    }
    cursor = Math.max(cursor, end);
  }
  if (cursor < line.lineText.length) {
    nodes.push(line.lineText.slice(cursor));
  }

  return <>{nodes}</>;
}

function fileNameOf(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? path;
}

function parentDirectoryOf(path: string): string | null {
  const segments = path.split(/[\\/]/).filter(Boolean);
  if (segments.length <= 1) return null;

  const parent = segments.slice(0, -1).join("\\");
  return /^[a-zA-Z]:$/.test(parent) ? `${parent}\\` : parent;
}

function getErrorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }

  return error instanceof Error ? error.message : String(error);
}
