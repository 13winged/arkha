import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";

interface CategoryPreview {
  id: string;
  title: string;
  description: string;
  files: number;
  bytes: number;
  conflicts: number;
}

interface MigrationPreview {
  pending: boolean;
  alreadyDecided: boolean;
  guest: boolean;
  browserPreferences: boolean;
  categories: CategoryPreview[];
}

interface Props {
  guest: boolean;
  children: ReactNode;
}

type GateState =
  | { phase: "loading" }
  | { phase: "ready" }
  | { phase: "prompt"; categories: CategoryPreview[]; language: string | null; fontSize: number | null }
  | { phase: "error"; message: string; categories?: CategoryPreview[] };

const FONT_SIZES = [13, 14, 15, 16, 18];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КБ`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} МБ`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} ГБ`;
}

export default function LegacyMigrationGate({ guest, children }: Props) {
  const [state, setState] = useState<GateState>({ phase: "loading" });
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function inspect() {
    setState({ phase: "loading" });
    try {
      const oldLanguage = localStorage.getItem("arkha-lang");
      const language = oldLanguage === "ru" || oldLanguage === "en" ? oldLanguage : null;
      const rawFont = Number(localStorage.getItem("arkha-font-size"));
      const fontSize = FONT_SIZES.includes(rawFont) ? rawFont : null;
      const preview = await invoke<MigrationPreview>("legacy_migration_preview", {
        legacyLanguage: language,
        legacyFontSize: fontSize,
      });
      if (preview.guest || preview.alreadyDecided || !preview.pending) {
        setState({ phase: "ready" });
        return;
      }
      const categories = [...preview.categories];
      if (preview.browserPreferences) {
        categories.push({
          id: "browser-preferences",
          title: "Настройки интерфейса браузера",
          description: "Старые язык и размер шрифта из WebView. Они применятся поверх настроек приложения, если выбраны обе категории.",
          files: 0,
          bytes: 0,
          conflicts: 0,
        });
      }
      const available = categories.filter((category) => category.conflicts === 0);
      setSelected(available.map((category) => category.id));
      setState({ phase: "prompt", categories, language, fontSize });
    } catch (reason) {
      setState({ phase: "error", message: String(reason) });
    }
  }

  useEffect(() => {
    if (guest) {
      setState({ phase: "ready" });
      return;
    }
    void inspect();
  }, [guest]);

  async function decideSkip() {
    setBusy(true);
    try {
      await invoke("legacy_migration_skip");
      setState({ phase: "ready" });
    } catch (reason) {
      setState({ phase: "error", message: String(reason) });
    } finally {
      setBusy(false);
    }
  }

  async function importSelected() {
    if (selected.length === 0) return;
    setBusy(true);
    try {
      await invoke("legacy_migration_import", {
        categoriesSelected: selected,
        legacyLanguage: selected.includes("browser-preferences") && state.phase === "prompt" ? state.language : null,
        legacyFontSize: selected.includes("browser-preferences") && state.phase === "prompt" ? state.fontSize : null,
      });
      setState({ phase: "ready" });
    } catch (reason) {
      setState((current) => ({
        phase: "error",
        message: String(reason),
        categories: current.phase === "prompt" ? current.categories : undefined,
      }));
    } finally {
      setBusy(false);
    }
  }

  if (state.phase === "ready") return <>{children}</>;
  if (state.phase === "loading") {
    return <main className="auth-screen"><p>{t("Проверяем старые данные…")}</p></main>;
  }

  const categories = state.phase === "prompt" ? state.categories : state.categories ?? [];
  const message = state.phase === "error" ? state.message : null;

  return (
    <main className="auth-screen migration-screen">
      <section className="migration-card" aria-labelledby="migration-title">
        <div className="brand-mark cp-glitch" data-glitch-text="ARKHA">ARKHA</div>
        <h1 id="migration-title">{t("Перенос старых данных")}</h1>
        <p>{t("Найдены данные старой версии. Выберите, что перенести в этот аккаунт.")}</p>
        <p className="migration-notice">{t("Исходные данные останутся на месте. После импорта legacy-набор будет закреплён за этим аккаунтом и не предложен другим аккаунтам. Общие модели и runtime не переносятся. Уже существующие файлы профиля не перезаписываются.")}</p>

        {categories.length > 0 && <fieldset className="migration-categories">
          <legend>{t("Категории для переноса")}</legend>
          {categories.map((category) => {
            const conflict = category.conflicts > 0;
            const checked = selected.includes(category.id);
            return (
              <label className={`migration-category ${conflict ? "conflict" : ""}`} key={category.id}>
                <input
                  type="checkbox"
                  checked={checked}
                  disabled={busy || conflict}
                  onChange={(event) => setSelected((current) => event.target.checked
                    ? [...current, category.id]
                    : current.filter((id) => id !== category.id))}
                />
                <span className="migration-category-copy">
                  <strong>{t(category.title)}</strong>
                  <span>{t(category.description)}</span>
                  {category.files > 0 && <small>{category.files} {t("файлов")} · {formatBytes(category.bytes)}</small>}
                  {conflict && <small className="migration-conflict">{t("Есть совпадения в профиле — категория пропущена без перезаписи.")}</small>}
                </span>
              </label>
            );
          })}
        </fieldset>}

        {message && <p className="error" role="alert">{message}</p>}
        <div className="migration-actions">
          {state.phase === "error" && <button type="button" onClick={() => void inspect()} disabled={busy}>{t("Повторить проверку")}</button>}
          <button type="button" className="migration-secondary" onClick={() => void decideSkip()} disabled={busy}>{t("Продолжить без переноса")}</button>
          {state.phase === "prompt" && <button type="button" onClick={() => void importSelected()} disabled={busy || selected.length === 0}>
            {busy ? t("Переносим…") : t("Перенести выбранное")}
          </button>}
        </div>
      </section>
    </main>
  );
}
