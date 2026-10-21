import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { CloseIcon, LoaderIcon, MagnifierIcon, TextFieldIcon } from "@solar-icons/react/linear";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";

import { commands } from "@/bindings";
import { translateBackendMessage } from "@/i18n/errors";
import { useAtomValue } from "jotai";
import { useHotkeys } from "@tanstack/react-hotkeys";
import { appSettingsAtom, hotkeysPausedAtom } from "@/features/settings/settings-atoms";
import { formatBinding, resolveBinding } from "@/features/settings/shortcut-registry";
import { HOTKEY_COMMON_OPTIONS, asHotkey } from "@/features/settings/hotkeys";

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
  const { t } = useTranslation("explorer");
  const inputRef = useRef<HTMLInputElement>(null);
  const [isFocused, setIsFocused] = useState(false);
  const isContentMode = mode === "content";
  const activeQuery = isContentMode ? contentSearch.query : search.query;
  const setActiveQuery = isContentMode ? contentSearch.setQuery : search.setQuery;
  const isSearching = isContentMode ? contentSearch.isSearching : search.isSearching;
  const activeError = isContentMode ? contentSearch.error : search.error;

  const shortcuts = useAtomValue(appSettingsAtom)?.shortcuts;
  const hotkeysPaused = useAtomValue(hotkeysPausedAtom);

  // Mod+F focuses and selects the search field. Inactive when this pane is not
  // the focused one, or while the shortcut recorder is capturing a new binding.
  useHotkeys(
    [
      {
        hotkey: asHotkey(resolveBinding(shortcuts, "explorer.focusSearch")),
        callback: (event) => {
          if (event.defaultPrevented || event.isComposing) return;
          event.preventDefault();
          inputRef.current?.focus();
          inputRef.current?.select();
        },
      },
    ],
    { ...HOTKEY_COMMON_OPTIONS, enabled: !disabled && !hotkeysPaused },
  );

  const scopeName = directoryName ?? t("directorySearch.currentDirectory");
  const focusBinding = formatBinding(resolveBinding(shortcuts, "explorer.focusSearch"));

  return (
    // `shrink` rather than `shrink-0`: the toolbar is one non-wrapping row, and
    // a fixed 224px field was the first thing to push the controls past the
    // pane's edge. `min-w-24` keeps the field usable down to the narrowest
    // window instead of letting it collapse to an icon-less sliver. It is the
    // same 36px capsule as the path field beside it — two fields in one row
    // that disagree about their height read as two different toolbars.
    <InputGroup
      // The field gives width back to the path bar while it is idle and takes
      // it when the user is actually typing: 176px at rest, 256px once the
      // caret is in. Below 1360px it folds to its magnifier — a search box
      // nobody is using must never be the reason the path is unreadable — and
      // unfolds again on focus, which is exactly when it is being used.
      className="h-8 w-44 min-w-0 shrink rounded-md shadow-[inset_0_0_0_1px_var(--border)] transition-[width] duration-normal ease-standard hover:w-56 focus-within:w-64 @max-[780px]:w-8 @max-[780px]:hover:w-8 @max-[780px]:focus-within:w-60"
    >
      <InputGroupInput
        // 13px on the reference's `.searchmini`, whose field is caption-sized
        // beside a 30px capsule; at the shell's 14.5px body step the query and
        // the folder name beside it argued about which was the label.
        className="text-caption"
        ref={inputRef}
        aria-invalid={Boolean(activeError)}
        aria-label={
          isContentMode
            ? t("directorySearch.contentAriaLabel", { scope: scopeName })
            : t("directorySearch.nameAriaLabel", { scope: scopeName })
        }
        disabled={disabled}
        onChange={(event) => setActiveQuery(event.target.value)}
        onBlur={() => setIsFocused(false)}
        onFocus={() => setIsFocused(true)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && activeQuery) {
            event.preventDefault();
            setActiveQuery("");
          }
        }}
        placeholder={
          isContentMode
            ? t("directorySearch.contentPlaceholder")
            : t("directorySearch.namePlaceholder", { scope: scopeName })
        }
        spellCheck={false}
        title={
          isContentMode
            ? t("directorySearch.contentTitle", { scope: scopeName })
            : t("directorySearch.nameTitle", { scope: scopeName })
        }
        value={activeQuery}
      />
      <InputGroupAddon align="inline-start">
        <InputGroupButton
          aria-label={
            isContentMode ? t("directorySearch.switchToName") : t("directorySearch.switchToContent")
          }
          onClick={() => onModeChange(isContentMode ? "name" : "content")}
          size="icon-xs"
          title={
            isContentMode
              ? t("directorySearch.currentContentTitle")
              : t("directorySearch.currentNameTitle")
          }
        >
          {/* The glyph is the field's primary affordance, so it carries full
              ink: the addon's muted step is meant for a trailing keycap hint,
              and at 12–14px it left the control looking disabled. */}
          {isContentMode ? (
            <TextFieldIcon className="text-foreground" />
          ) : (
            <MagnifierIcon className="text-foreground" />
          )}
        </InputGroupButton>
      </InputGroupAddon>
      {(isSearching || activeQuery) && (
        <InputGroupAddon align="inline-end">
          {isSearching && <LoaderIcon className="animate-spin" />}
          {activeQuery && (
            <InputGroupButton
              aria-label={t("directorySearch.clearSearch")}
              onClick={() => setActiveQuery("")}
              size="icon-xs"
              title={t("directorySearch.clearSearch")}
            >
              <CloseIcon className="text-foreground" />
            </InputGroupButton>
          )}
        </InputGroupAddon>
      )}
      {/* Idle, unfocused, empty: the field has nothing to say about its own
          state, so it says how to reach itself instead. The hint is dropped as
          soon as the caret is in the box — at that point the key has done its
          job and the chip would only crowd the placeholder. */}
      {!isFocused && !isSearching && !activeQuery && (
        // Dropped where the field folds to its magnifier: outside a 176px box
        // the keycap is the widest thing in it, and a chip hanging out of a
        // 36px control reads as a rendering bug.
        <InputGroupAddon align="inline-end" className="@max-[780px]:hidden">
          <Kbd className="h-4 px-1 text-nano">{focusBinding}</Kbd>
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
    return translateBackendMessage(error.message);
  }

  return error instanceof Error ? translateBackendMessage(error.message) : String(error);
}
