import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { CircleNotchIcon, MagnifyingGlassIcon, XIcon } from "@phosphor-icons/react";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";

import { commands } from "@/bindings";

import type { SearchResponse } from "./types";

const SEARCH_DEBOUNCE_MS = 180;

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

    if (!directoryPath || !trimmedQuery) {
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
  }, [directoryPath, refreshToken, trimmedQuery]);

  return {
    error,
    isActive: trimmedQuery.length > 0,
    isSearching,
    query,
    response,
    setQuery,
  };
}

export function DirectorySearch({
  directoryName,
  disabled,
  search,
}: {
  directoryName: string | null;
  disabled: boolean;
  search: DirectorySearchController;
}) {
  const inputRef = useRef<HTMLInputElement>(null);

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
    <InputGroup className="w-44 shrink-0 sm:w-56">
      <InputGroupInput
        ref={inputRef}
        aria-invalid={Boolean(search.error)}
        aria-label={`搜索“${scopeName}”中的文件和文件夹`}
        disabled={disabled}
        onChange={(event) => search.setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && search.query) {
            event.preventDefault();
            search.setQuery("");
          }
        }}
        placeholder={`搜索 ${scopeName}`}
        spellCheck={false}
        title={search.error ?? `递归搜索“${scopeName}”`}
        value={search.query}
      />
      <InputGroupAddon align="inline-start">
        <MagnifyingGlassIcon />
      </InputGroupAddon>
      {(search.isSearching || search.query) && (
        <InputGroupAddon align="inline-end">
          {search.isSearching && <CircleNotchIcon className="animate-spin" />}
          {search.query && (
            <InputGroupButton
              aria-label="清除搜索"
              onClick={() => search.setQuery("")}
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
