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

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useAtom, useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import {
  CircleCheck,
  LoaderCircle,
  Settings,
  Keyboard,
  Minus,
  Palette,
  Plus,
  ScrollText,
  SquareTerminal,
} from "lucide-react";
import { appLogDir } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";

import { commands, type DefaultFileManagerStatus } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";
import { SUPPORTED_LOCALES, type AppLocale } from "@/i18n";
import { localeAtom } from "@/i18n/atoms";
import { cn } from "@/lib/utils";
import { getStoredThemePreference, setThemePreference, type ThemePreference } from "@/lib/theme";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
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

type Pane = "appearance" | "shortcuts" | "terminal" | "defaultFileManager" | "logs";

const NAV_ITEMS: ReadonlyArray<{ icon: typeof Settings; pane: Pane }> = [
  { icon: Palette, pane: "appearance" },
  { icon: Keyboard, pane: "shortcuts" },
  { icon: SquareTerminal, pane: "terminal" },
  { icon: Settings, pane: "defaultFileManager" },
  { icon: ScrollText, pane: "logs" },
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
      <DialogContent className="gap-0 overflow-hidden p-0 sm:max-w-3xl">
        <DialogTitle className="sr-only">{t("dialog.title")}</DialogTitle>
        {/* The body caps below the dialog's own max-height, so a short window
            shrinks the two panes instead of scrolling the whole dialog out
            from under its rounded frame. */}
        <div className="flex h-settings-body max-h-[calc(100dvh-4rem)]">
          <nav
            aria-label={t("dialog.navAria")}
            className="flex w-48 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border bg-muted p-2"
          >
            {NAV_ITEMS.map(({ icon: Icon, pane: item }) => (
              <button
                aria-current={pane === item}
                className={cn(
                  "flex h-7 items-center gap-2 rounded-sm px-2 text-body font-medium transition-colors duration-fast ease-standard outline-none",
                  pane === item
                    ? "bg-accent text-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground",
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
          <div className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
            {pane === "appearance" && <AppearancePane />}
            {pane === "shortcuts" && <ShortcutsPane />}
            {pane === "terminal" && <TerminalPane />}
            {pane === "defaultFileManager" && <DefaultFileManagerPane />}
            {pane === "logs" && <LogsPane />}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Settings pane title: `text-title` heading over one `text-caption` line, the
 * same header shape every other surface uses.
 */
function PaneHeader({ description, title }: { description: string; title: string }) {
  return (
    <header className="mb-5">
      <h2 className="text-title">{title}</h2>
      <p className="mt-0.5 text-caption text-muted-foreground">{description}</p>
    </header>
  );
}

/**
 * A block of setting rows divided by hairlines rather than boxed into cards —
 * the settings body has no second surface to spare, and the rows already
 * carry the rhythm.
 */
function SettingRows({ children }: { children: ReactNode }) {
  return <div className="flex flex-col">{children}</div>;
}

/**
 * One Linear-style setting row: label and description on the left, the control
 * hard right. The row is the unit of rhythm here, so a pane of settings reads
 * as a list rather than as a stack of labelled fields.
 */
function SettingRow({
  control,
  description,
  htmlFor,
  label,
}: {
  control: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  label: string;
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-border py-3 last:border-b-0">
      <div className="min-w-0">
        <Label className="text-body font-medium" htmlFor={htmlFor}>
          {label}
        </Label>
        {description && (
          <p className="mt-0.5 text-caption text-muted-foreground">{description}</p>
        )}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
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
    <div className="flex flex-col">
      <PaneHeader description={t("appearance.description")} title={t("nav.appearance")} />

      <SettingRows>
        <SettingRow
          control={
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
          }
          htmlFor="settings-appearance-theme"
          label={t("appearance.theme")}
        />
        <SettingRow
          control={
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
          }
          htmlFor="settings-appearance-language"
          label={t("appearance.language")}
        />
      </SettingRows>
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
    <div className="flex flex-col">
      <header className="mb-5 flex items-center justify-between gap-3">
        <div>
          <h2 className="text-title">{t("nav.shortcuts")}</h2>
          <p className="mt-0.5 text-caption text-muted-foreground">
            {t("shortcuts.description")}
          </p>
        </div>
        <Button disabled={!isCustomized} onClick={resetAll} size="sm" variant="outline">
          {t("shortcuts.resetAll")}
        </Button>
      </header>
      {GROUP_ORDER.map((group) => {
        const actions = SHORTCUT_ACTIONS.filter((action) => action.group === group);
        if (actions.length === 0) return null;
        return (
          <section className="mb-4 flex flex-col last:mb-0" key={group}>
            <h3 className="mb-1 text-label text-muted-foreground uppercase">
              {t(`groups.${group}`)}
            </h3>
            {actions.map((action) => (
              <div
                className="flex min-h-8 items-center justify-between gap-4 rounded-sm px-2 text-body transition-colors duration-fast ease-standard hover:bg-accent"
                key={action.id}
              >
                <span>{t(`actions.${action.id}`)}</span>
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
    <div className="flex flex-col">
      <PaneHeader description={t("terminal.description")} title={t("nav.terminal")} />

      <SettingRows>
        <SettingRow
          control={
            <Input
              className="w-64"
              id="settings-terminal-font-family"
              onBlur={(event) => commitFontFamily(event.target.value)}
              onChange={(event) => setFontFamily(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter")
                  commitFontFamily((event.target as HTMLInputElement).value);
              }}
              placeholder={t("terminal.fontFamilyPlaceholder")}
              value={fontFamily}
            />
          }
          description={t("terminal.fontFamilyHint")}
          htmlFor="settings-terminal-font-family"
          label={t("terminal.fontFamily")}
        />
        <SettingRow
          control={
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
          }
          htmlFor="settings-terminal-font-size"
          label={t("terminal.fontSize")}
        />
        <SettingRow
          control={
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
                  <Minus />
                </InputGroupButton>
                <InputGroupButton
                  aria-label={t("terminal.lineHeightIncrease")}
                  onClick={() => stepLineHeight(0.1)}
                >
                  <Plus />
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
          }
          description={t("terminal.lineHeightHint")}
          htmlFor="settings-terminal-line-height"
          label={t("terminal.lineHeight")}
        />
      </SettingRows>
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
    <div className="flex flex-col">
      <PaneHeader
        description={t("defaultFileManager.description")}
        title={t("nav.defaultFileManager")}
      />

      {!supported ? (
        <p className="text-body text-muted-foreground">{t("defaultFileManager.unsupported")}</p>
      ) : (
        <>
          <div className="flex items-center gap-2 text-body">
            {busy ? (
              <LoaderCircle className="size-4 animate-spin text-muted-foreground" />
            ) : isDefault ? (
              <CircleCheck className="size-4 text-success" />
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
            <p className="text-caption text-muted-foreground">
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

      {error && <p className="text-caption text-destructive">{error}</p>}
    </div>
  );
}

/**
 * Where the log file is, and the button that opens it.
 *
 * The app never uploads the log anywhere — it holds paths the user may consider
 * private, so reading it stays their decision, and sending it stays a manual
 * step they take.
 */
function LogsPane() {
  const { t } = useTranslation("settings");
  const [error, setError] = useState<string | null>(null);

  const openLogDirectory = useCallback(async () => {
    setError(null);
    try {
      // The backend's `LogDir` target resolves the same directory through the
      // same Tauri API. `appLogDir` rides `path|resolve_directory`, which
      // `core:path:default` already grants, so no capability entry is needed.
      await openPath(await appLogDir());
    } catch (reason) {
      setError(getFileOperationErrorMessage(reason));
    }
  }, []);

  return (
    <div className="flex flex-col">
      <PaneHeader description={t("logs.description")} title={t("nav.logs")} />

      <div className="flex flex-col gap-2">
        <Button className="self-start" onClick={() => void openLogDirectory()} variant="outline">
          {t("logs.openDirectory")}
        </Button>
        <p className="text-caption text-muted-foreground">{t("logs.privacy")}</p>
      </div>

      {error && <p className="text-caption text-destructive">{error}</p>}
    </div>
  );
}
