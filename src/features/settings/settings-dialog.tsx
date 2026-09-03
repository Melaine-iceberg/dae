/**
 * Application settings dialog (SKILL: lazy-mounted overlay).
 *
 * Two-column layout: a compact nav rail (Shortcuts / Terminal / Default file
 * manager) and the active pane. The dialog is mounted on demand from App.tsx
 * via {@link settingsOpenAtom}; it reads and writes the hydrated
 * {@link appSettingsAtom} through `useSettings()` (optimistic + backend save).
 *
 * Scope note: shortcuts, terminal, and default-FM live in the TOML store.
 * Theme and language are backed by localStorage and are surfaced here through
 * the Appearance pane.
 */

import { useCallback, useEffect, useState } from "react";
import { useAtom, useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import {
  CheckCircleIcon,
  CircleNotchIcon,
  GearIcon,
  KeyboardIcon,
  MinusIcon,
  PaletteIcon,
  PlusIcon,
  TerminalIcon,
} from "@phosphor-icons/react";

import { commands, type DefaultFileManagerStatus } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { SUPPORTED_LOCALES, type AppLocale } from "@/i18n";
import { localeAtom } from "@/i18n/atoms";
import { cn } from "@/lib/utils";
import {
  getStoredThemePreference,
  setThemePreference,
  type ThemePreference,
} from "@/lib/theme";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import { settingsOpenAtom, useSettings } from "./settings-atoms";
import { ShortcutRecorder } from "./shortcut-recorder";
import {
  DEFAULT_BINDINGS,
  SHORTCUT_ACTIONS,
  resolveBinding,
  type ShortcutGroup,
  type ShortcutId,
} from "./shortcut-registry";

type Pane = "appearance" | "shortcuts" | "terminal" | "defaultFileManager";

const NAV_ITEMS: ReadonlyArray<{ icon: typeof GearIcon; pane: Pane }> = [
  { icon: PaletteIcon, pane: "appearance" },
  { icon: KeyboardIcon, pane: "shortcuts" },
  { icon: TerminalIcon, pane: "terminal" },
  { icon: GearIcon, pane: "defaultFileManager" },
];

const GROUP_ORDER: readonly ShortcutGroup[] = ["app", "explorer", "view"];

const FONT_SIZE_OPTIONS = [11, 12, 13, 14, 15, 16, 18, 20, 24];

const THEME_OPTIONS: readonly ThemePreference[] = ["light", "dark", "system"];

export function SettingsDialog() {
  const { t } = useTranslation("settings");
  const [open, setOpen] = useAtom(settingsOpenAtom);
  const [pane, setPane] = useState<Pane>("appearance");

  return (
    <Dialog onOpenChange={setOpen} open={open}>
      <DialogContent className="gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <DialogTitle className="sr-only">{t("dialog.title")}</DialogTitle>
        <div className="flex h-[30rem] max-h-[calc(100vh-4rem)]">
          <nav
            aria-label={t("dialog.navAria")}
            className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-border bg-muted/30 p-2"
          >
            {NAV_ITEMS.map(({ icon: Icon, pane: item }) => (
              <button
                aria-current={pane === item}
                className={cn(
                  "flex h-8 items-center gap-2 rounded-sm px-2 text-[13px] font-medium transition-colors outline-none",
                  pane === item
                    ? "bg-accent text-foreground"
                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                )}
                key={item}
                onClick={() => setPane(item)}
                type="button"
              >
                <Icon className="size-4" />
                {t(`nav.${item}`)}
              </button>
            ))}
          </nav>
          <div className="min-w-0 flex-1 overflow-y-auto p-5">
            {pane === "appearance" && <AppearancePane />}
            {pane === "shortcuts" && <ShortcutsPane />}
            {pane === "terminal" && <TerminalPane />}
            {pane === "defaultFileManager" && <DefaultFileManagerPane />}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function AppearancePane() {
  const { t } = useTranslation("settings");
  const [theme, setTheme] = useState<ThemePreference>(() => getStoredThemePreference());
  const [locale, setLocale] = useAtom(localeAtom);

  // Keep in sync when the theme changes elsewhere (e.g. an OS scheme flip
  // while "system" is selected).
  useEffect(() => {
    const sync = () => setTheme(getStoredThemePreference());
    window.addEventListener("app-theme-change", sync);
    return () => window.removeEventListener("app-theme-change", sync);
  }, []);

  return (
    <div className="flex flex-col gap-5">
      <header>
        <h2 className="font-heading text-sm font-semibold">{t("nav.appearance")}</h2>
        <p className="text-xs text-muted-foreground">{t("appearance.description")}</p>
      </header>

      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-appearance-theme">{t("appearance.theme")}</Label>
        <Select
          items={Object.fromEntries(
            THEME_OPTIONS.map((option) => [option, t(`appearance.themeOptions.${option}`)]),
          )}
          onValueChange={(value) => setThemePreference(value as ThemePreference)}
          value={theme}
        >
          <SelectTrigger className="w-40" id="settings-appearance-theme">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {THEME_OPTIONS.map((option) => (
              <SelectItem key={option} value={option}>
                {t(`appearance.themeOptions.${option}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-appearance-language">{t("appearance.language")}</Label>
        <Select
          items={Object.fromEntries(
            SUPPORTED_LOCALES.map((value) => [value, t(`common:language.${value}`)]),
          )}
          onValueChange={(value) => setLocale(value as AppLocale)}
          value={locale}
        >
          <SelectTrigger className="w-40" id="settings-appearance-language">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SUPPORTED_LOCALES.map((value) => (
              <SelectItem key={value} value={value}>
                {t(`common:language.${value}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

function ShortcutsPane() {
  const { t } = useTranslation("settings");
  const [settings, patch] = useSettings();
  const shortcuts = settings.shortcuts;

  const commit = useCallback(
    (id: ShortcutId, next: string) => patch({ shortcuts: { [id]: next } }),
    [patch],
  );
  const resetAll = useCallback(() => {
    const restored = Object.fromEntries(
      (Object.keys(DEFAULT_BINDINGS) as ShortcutId[]).map((id) => [id, DEFAULT_BINDINGS[id]]),
    );
    patch({ shortcuts: restored });
  }, [patch]);

  const isCustomized = (Object.keys(DEFAULT_BINDINGS) as ShortcutId[]).some(
    (id) => resolveBinding(shortcuts, id) !== DEFAULT_BINDINGS[id],
  );

  return (
    <div className="flex flex-col gap-5">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h2 className="font-heading text-sm font-semibold">{t("nav.shortcuts")}</h2>
          <p className="text-xs text-muted-foreground">{t("shortcuts.description")}</p>
        </div>
        <Button disabled={!isCustomized} onClick={resetAll} size="sm" variant="outline">
          {t("shortcuts.resetAll")}
        </Button>
      </header>
      {GROUP_ORDER.map((group) => {
        const actions = SHORTCUT_ACTIONS.filter((action) => action.group === group);
        if (actions.length === 0) return null;
        return (
          <section className="flex flex-col gap-1" key={group}>
            <h3 className="mb-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
              {t(`groups.${group}`)}
            </h3>
            {actions.map((action) => (
              <div
                className="flex items-center justify-between gap-4 rounded-sm px-1 py-1.5 hover:bg-accent/40"
                key={action.id}
              >
                <span className="text-[13px]">{t(`actions.${action.id}`)}</span>
                <ShortcutRecorder
                  binding={resolveBinding(shortcuts, action.id)}
                  id={action.id}
                  onCommit={(next) => commit(action.id, next)}
                  onReset={() => commit(action.id, DEFAULT_BINDINGS[action.id])}
                />
              </div>
            ))}
          </section>
        );
      })}
    </div>
  );
}

function TerminalPane() {
  const { t } = useTranslation("settings");
  const [settings, patch] = useSettings();
  const terminal = settings.terminal;
  const [fontFamily, setFontFamily] = useState(terminal?.fontFamily ?? "");

  // Keep the free-text input in sync when settings change elsewhere (e.g. a
  // backend reload), without fighting the user mid-typing.
  useEffect(() => {
    setFontFamily(terminal?.fontFamily ?? "");
  }, [terminal?.fontFamily]);

  const commitFontFamily = (value: string) => {
    const trimmed = value.trim();
    patch({ terminal: { fontFamily: trimmed.length > 0 ? trimmed : null } });
  };

  const stepLineHeight = (delta: number) => {
    const current = terminal?.lineHeight ?? 1.2;
    const next = Math.min(3, Math.max(0.8, Math.round((current + delta) * 10) / 10));
    patch({ terminal: { lineHeight: next } });
  };

  return (
    <div className="flex flex-col gap-5">
      <header>
        <h2 className="font-heading text-sm font-semibold">{t("nav.terminal")}</h2>
        <p className="text-xs text-muted-foreground">{t("terminal.description")}</p>
      </header>

      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-terminal-font-family">{t("terminal.fontFamily")}</Label>
        <Input
          id="settings-terminal-font-family"
          onBlur={(event) => commitFontFamily(event.target.value)}
          onChange={(event) => setFontFamily(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commitFontFamily((event.target as HTMLInputElement).value);
          }}
          placeholder={t("terminal.fontFamilyPlaceholder")}
          value={fontFamily}
        />
        <p className="text-xs text-muted-foreground">{t("terminal.fontFamilyHint")}</p>
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-terminal-font-size">{t("terminal.fontSize")}</Label>
        <Select
          items={Object.fromEntries(FONT_SIZE_OPTIONS.map((size) => [size, `${size} px`]))}
          onValueChange={(value) => patch({ terminal: { fontSize: Number(value) } })}
          value={terminal?.fontSize ?? 13}
        >
          <SelectTrigger className="w-40" id="settings-terminal-font-size">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {FONT_SIZE_OPTIONS.map((size) => (
              <SelectItem key={size} value={size}>
                {size} px
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-terminal-line-height">{t("terminal.lineHeight")}</Label>
        <InputGroup className="w-40">
          <InputGroupInput
            id="settings-terminal-line-height"
            max={3}
            min={0.8}
            onChange={(event) => {
              const value = Number(event.target.value);
              if (Number.isFinite(value)) patch({ terminal: { lineHeight: value } });
            }}
            step={0.1}
            type="number"
            value={terminal?.lineHeight ?? 1.2}
          />
          <InputGroupAddon align="inline-end">
            <InputGroupButton
              aria-label={t("terminal.lineHeightDecrease")}
              onClick={() => stepLineHeight(-0.1)}
            >
              <MinusIcon />
            </InputGroupButton>
            <InputGroupButton
              aria-label={t("terminal.lineHeightIncrease")}
              onClick={() => stepLineHeight(0.1)}
            >
              <PlusIcon />
            </InputGroupButton>
          </InputGroupAddon>
        </InputGroup>
        <p className="text-xs text-muted-foreground">{t("terminal.lineHeightHint")}</p>
      </div>
    </div>
  );
}

function DefaultFileManagerPane() {
  const { t } = useTranslation("settings");
  const [, patch] = useSettings();
  const dialogOpen = useAtomValue(settingsOpenAtom);
  const [status, setStatus] = useState<DefaultFileManagerStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-read the OS state each time the dialog opens, not just on first mount,
  // since the user may have confirmed the choice in system settings meanwhile.
  useEffect(() => {
    if (!dialogOpen) return;
    let cancelled = false;
    setBusy(true);
    void commands
      .getDefaultFileManagerStatus()
      .then((next) => {
        if (!cancelled) setStatus(next);
      })
      .catch((reason) => {
        if (!cancelled) setError(getFileOperationErrorMessage(reason));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [dialogOpen]);

  const apply = useCallback(
    async (makeDefault: boolean) => {
      setBusy(true);
      setError(null);
      try {
        const next = makeDefault
          ? await commands.setDefaultFileManager()
          : await commands.unsetDefaultFileManager();
        setStatus(next);
        patch({ defaultFileManager: { isDefault: next.isDefault } });
      } catch (reason) {
        setError(getFileOperationErrorMessage(reason));
      } finally {
        setBusy(false);
      }
    },
    [patch],
  );

  const supported = status?.supported ?? true;
  const isDefault = status?.isDefault ?? false;

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="font-heading text-sm font-semibold">{t("nav.defaultFileManager")}</h2>
        <p className="text-xs text-muted-foreground">{t("defaultFileManager.description")}</p>
      </header>

      {!supported ? (
        <p className="text-[13px] text-muted-foreground">
          {t("defaultFileManager.unsupported")}
        </p>
      ) : (
        <>
          <div className="flex items-center gap-2 text-[13px]">
            {busy ? (
              <CircleNotchIcon className="size-4 animate-spin text-muted-foreground" />
            ) : isDefault ? (
              <CheckCircleIcon className="size-4 text-icon-sheet" />
            ) : null}
            <span>
              {isDefault
                ? t("defaultFileManager.statusDefault")
                : status?.isRegistered
                  ? t("defaultFileManager.statusRegistered")
                  : t("defaultFileManager.statusNotDefault")}
            </span>
          </div>

          {status?.detail && (
            <p className="text-xs text-muted-foreground">
              {t(status.detail, { defaultValue: status.detail })}
            </p>
          )}

          <div className="flex items-center gap-2">
            <Button disabled={busy || isDefault} onClick={() => void apply(true)} variant="default">
              {t("defaultFileManager.setDefault")}
            </Button>
            <Button
              disabled={busy || !isDefault}
              onClick={() => void apply(false)}
              variant="outline"
            >
              {t("defaultFileManager.clearDefault")}
            </Button>
          </div>
        </>
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}
