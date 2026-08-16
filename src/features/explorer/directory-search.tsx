import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  CircleNotchIcon,
  MagnifyingGlassIcon,
  TextAaIcon,
  XIcon,
} from "@phosphor-icons/react";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";

import { commands } from "@/bindings";

import type { ContentSearchController } from "./content-search";
import type { SearchResponse } from "./types";

const SEARCH_DEBOUNCE_MS = 180;

export type ExplorerSearchMode = "name" | "content";

export interface DirectorySearchController {
  error: string | null;
  isActive: boolean;
  isSearching: boolean;
  query: string;
  response: SearchResponse | null;
  setQuery: Dispatch<SetStateAction<string>>;
}

export function useDirectorySearch(
  directoryPath: string | null,
  refreshToken: object | null,
  enabled = true,
): DirectorySearchController {
  const requestVersionRef = useRef(0);
  const [query, setQuery] = useState("");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmedQuery = query.trim();

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
      if (directoryPath && !trimmedQuery) {
        void commands.cancelSearch().catch(() => undefined);
      }
      return;
    }

    setIsSearching(true);
    const timeout = window.setTimeout(() => {
      void commands
        .searchDirectory(directoryPath, trimmedQuery)
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
    }, SEARCH_DEBOUNCE_MS);

    return () => window.clearTimeout(timeout);
  }, [directoryPath, enabled, refreshToken, trimmedQuery]);

  return {
    error,
    isActive: enabled && trimmedQuery.length > 0,
    isSearching,
    query,
    response,
    setQuery,
  };
}

export function DirectorySearch({
  contentSearch,
  directoryName,
  disabled,
  mode,
  onModeChange,
  search,
}: {
  contentSearch: ContentSearchController;
  directoryName: string | null;
  disabled: boolean;
  mode: ExplorerSearchMode;
  onModeChange: (mode: ExplorerSearchMode) => void;
  search: DirectorySearchController;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const isContentMode = mode === "content";
  const activeQuery = isContentMode ? contentSearch.query : search.query;
  const setActiveQuery = isContentMode ? contentSearch.setQuery : search.setQuery;
  const isSearching = isContentMode ? contentSearch.isSearching : search.isSearching;
  const activeError = isContentMode ? contentSearch.error : search.error;

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const isSearchShortcut =
        (event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "f";

      if (event.defaultPrevented || event.isComposing || !isSearchShortcut || disabled) {
        return;
      }

      event.preventDefault();
      inputRef.current?.focus();
      inputRef.current?.select();
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [disabled]);

  const scopeName = directoryName ?? "当前目录";

  return (
    <InputGroup className="w-56 shrink-0 rounded-full">
      <InputGroupInput
        ref={inputRef}
        aria-invalid={Boolean(activeError)}
        aria-label={
          isContentMode
            ? `搜索“${scopeName}”中的文件内容`
            : `搜索“${scopeName}”中的文件和文件夹`
        }
        disabled={disabled}
        onChange={(event) => setActiveQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && activeQuery) {
            event.preventDefault();
            setActiveQuery("");
          }
        }}
        placeholder={isContentMode ? `在内容中搜索` : `搜索 ${scopeName}`}
        spellCheck={false}
        title={
          isContentMode
            ? `递归搜索“${scopeName}”中的文件内容（正则可选）`
            : `递归搜索“${scopeName}”的文件名`
        }
        value={activeQuery}
      />
      <InputGroupAddon align="inline-start">
        <InputGroupButton
          aria-label={isContentMode ? "切换为按名称搜索" : "切换为按内容搜索"}
          onClick={() => onModeChange(isContentMode ? "name" : "content")}
          size="icon-xs"
          title={isContentMode ? "当前：按内容搜索，点击切换为按名称" : "当前：按名称搜索，点击切换为按内容"}
        >
          {isContentMode ? <TextAaIcon /> : <MagnifyingGlassIcon />}
        </InputGroupButton>
      </InputGroupAddon>
      {(isSearching || activeQuery) && (
        <InputGroupAddon align="inline-end">
          {isSearching && <CircleNotchIcon className="animate-spin" />}
          {activeQuery && (
            <InputGroupButton
              aria-label="清除搜索"
              onClick={() => setActiveQuery("")}
              size="icon-xs"
              title="清除搜索"
            >
              <XIcon />
            </InputGroupButton>
          )}
        </InputGroupAddon>
      )}
    </InputGroup>
  );
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
