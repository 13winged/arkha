import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t, язык, применитьЯзык, следить } from "./i18n";
import Agent from "./Agent";
import AgentSelect from "./AgentSelect";
import Dashboard from "./Dashboard";
import RagWindow from "./RagWindow";
import LegacyMigrationGate from "./LegacyMigrationGate";
import "./App.css";

// Реестр возможностей. Причина недоступности приходит готовой с
// Rust: если позволить интерфейсу решать, он рано или поздно
// покажет «недоступно» без причины, и это будет наша ложь.
export interface CapabilityRow {
  id: string;
  name: string;
  what: string;
  available: boolean;
  blocked_by: string | null;
  refusal: string | null;
  evidence: string | null;
  min_tier: string;
}

export interface CapabilitiesReport {
  tier: string;
  total: number;
  available: number;
  blocked: number;
  verified: number;
  rows: CapabilityRow[];
}

export interface OnboardingStep {
  step: string;
  title: string;
  index: number;
  total: number;
  what: string;
  prompt: string;
}

export interface OnboardingSay {
  understood: boolean;
  next: OnboardingStep;
  summary: string;
  finished: boolean;
}

export interface SettingsView {
  executor: { value: string; title: string; explain: string };
  approval_mode: { value: string; title: string; explain: string };
  cloud: boolean;
  deny_apps: string[];
  paths: string[];
  disabled: string[];
  warning: string | null;
}

export interface OnboardingPreview {
  summary: string;
  settings: SettingsView;
}

export interface FolderView {
  label: string;
  path: string;
}

/** Одна подсказка из поиска моделей. */
export interface ModelHit {
  repo: string;
  family: string;
  downloads: number;
}

/// Выбор из списка. Один компонент на инструменты и папки: логика
/// «отметил, подтвердил» одинакова, различаются только подписи.
function Picker({
  items,
  chosen,
  onToggle,
  onDone,
  busy,
  empty,
}: {
  items: { id: string; title: string; note: string; tag?: string | null }[];
  chosen: Set<string>;
  onToggle: (id: string, want: boolean) => void;
  onDone: () => void;
  busy: boolean;
  empty: string;
}) {
  return (
    <div className="picker">
      {items.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        items.map((r) => {
          const on = chosen.has(r.id);
          return (
            <button
              key={r.id}
              type="button"
              className={on ? "pick pick-on" : "pick"}
              onClick={() => onToggle(r.id, !on)}
              disabled={busy}
            >
              <span className="pick-box">{on ? "✓" : ""}</span>
              <span className="pick-body">
                <span className="pick-name">
                  {r.title}
                  {r.tag && <span className="cap-tag cap-tag-no">{r.tag}</span>}
                </span>
                <span className="pick-what">{r.note}</span>
              </span>
            </button>
          );
        })
      )}
      {items.length > 0 && (
        <div className="onb-input">
          <button type="button" onClick={onDone} disabled={busy}>{t("дальше")}
          </button>
        </div>
      )}
    </div>
  );
}

// Выбор инструментов кнопками. Человек не должен писать JSON, чтобы
// отметить галочку, поэтому выбор приходит списком, а не фразой.
function ToolPicker({
  rows,
  chosen,
  onToggle,
  onDone,
  busy,
}: {
  rows: CapabilityRow[];
  chosen: Set<string>;
  onToggle: (id: string, want: boolean) => void;
  onDone: () => void;
  busy: boolean;
}) {
  return (
    <>
      <p className="muted">{t("Отметь, что секретарю можно делать. Недоступное сейчас всё равно отметить можно — включишь, когда появится.")}

      </p>
      <Picker
        items={rows.map((r) => ({
          id: r.id,
          title: r.name,
          note: r.what,
          tag: r.available ? null : r.blocked_by,
        }))}
        chosen={chosen}
        onToggle={onToggle}
        onDone={onDone}
        busy={busy}
        empty={t("Возможностей в реестре нет.")}
      />
    </>
  );
}

// Профиль приходит с Rust. Пока его нет — пустой экран, без «догадок».
export interface TierNote {
  tier: string;
  describe: string;
  blocked_because: string | null;
}

export interface ProfileView {
  summary: string;
  detected: string;
  effective: string;
  forced: string | null;
  model_name: string;
  model_quant: string;
  context: number;
  kv_cache_type: string;
  gpu_name: string;
  vram_mb: number;
  vram_usable_mb: number;
  is_discrete: boolean;
  hybrid: boolean;
  ram_mb: number;
  logical_cores: number;
  cpu_brand: string;
  detection_source: string;
  unsupported_reason: string | null;
  available_tiers: TierNote[];
}

export interface ModelStatus {
  running: boolean;
  recipe: string | null;
  kv_cache: string | null;
  context: number | null;
  tok_s: number | null;
  verified: boolean;
  full_gpu: boolean;
  gpu_layers: number | null;
  total_layers: number | null;
  device: string | null;
  flash_attn: boolean;
  error: string | null;
}

interface Message {
  role: "user" | "assistant";
  content: string;
  toolCalls?: unknown;
}

interface AttachmentView {
  id: string;
  name: string;
  mime: string;
  sizeBytes: number;
  path: string;
  indexed: boolean;
}

function gb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} ГБ`;
}

// ---------------------------------------------------------------------------
// Экран профиля (S1)
// ---------------------------------------------------------------------------

/// Что секретарь умеет и чего не умеет, с причинами.
///
/// Экран существует не для красоты. Пока возможностей мало, список
/// читается как отчёт о недоделке — и это честнее, чем интерфейс,
/// где всё выглядит работающим, а на деле доступен разговор.
function Capabilities() {
  const [report, setReport] = useState<CapabilitiesReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<CapabilitiesReport>("capabilities_report", { lang: язык() })
      .then(setReport)
      .catch((e) => setError(String(e)));
    // Отчёт перезапрашивается при смене языка: он приходит из Rust
    // уже переведённым, и без повторного вызова переключатель оставит
    // на экране текст на прежнем языке.
  }, [язык()]);

  if (error) {
    return (
      <div className="panel">
        <h2>{t("Возможности")}</h2>
        <p className="error">{error}</p>
      </div>
    );
  }

  if (!report) {
    return (
      <div className="panel">
        <h2>{t("Возможности")}</h2>
        <p className="muted">{t("Читаем реестр…")}</p>
      </div>
    );
  }

  const ready = report.rows.filter((r) => r.available);
  const blocked = report.rows.filter((r) => !r.available);

  return (
    <div className="panel">
      <h2>{t("Возможности")}</h2>

      <div className="stat-row">
        <span className="stat"><b>{report.available}</b> {t("работает")}</span>
        <span className="stat"><b>{report.total - report.available}</b> {t("закрыто")}</span>
        <span className="stat"><b>{report.verified}</b> {t("проверено на железе")}</span>
        <span className="stat gold"><b>{report.tier}</b> {t("профиль")}</span>
      </div>

      <section className="sector">
        <h3 className="sector-head">{t("работает")}
          <span className="sector-count">{ready.length}</span>
        </h3>
        {ready.length === 0 ? (
          <p className="muted">{t("Ничего не проверено. Это значит «ещё не сделано», а не «сломалось».")}</p>
        ) : (
          <div className="cards">
            {ready.map((r) => (
              <article key={r.id} className="card ok">
                <header className="card-head">
                  <span className="card-name">{r.name}</span>
                  <span className="cap-tag cap-tag-ok">{t("работает")}</span>
                </header>
                <p className="card-what">{r.what}</p>
                {r.evidence && <p className="card-note">{t("Проверено")}: {r.evidence}</p>}
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="sector">
        <h3 className="sector-head">{t("пока недоступно")}
          <span className="sector-count dim">{blocked.length}</span>
        </h3>
        <p className="muted sector-lead">{t("Секретарь знает про эти возможности и честно скажет, что не может. Не молчит и не обещает.")}

        </p>
        <div className="cards">
          {blocked.map((r) => (
            <article key={r.id} className="card no">
              <header className="card-head">
                <span className="card-name">{r.name}</span>
                <span className="cap-tag cap-tag-no">{r.blocked_by}</span>
              </header>
              <p className="card-what">{r.what}</p>
              {r.refusal && <p className="card-refusal">{r.refusal}</p>}
            </article>
          ))}
        </div>
      </section>
    </div>
  );
}

// ---------------------------------------------------------------------------

/// Знакомство: пользователь собирает секретаря в разговоре.
///
/// Экран намеренно показывает, что будет применено, до того как
/// применяет. Разрешения нельзя отменить задним числом, и человек
/// должен видеть, что подтверждает.
function Welcome() {
  const [step, setStep] = useState<OnboardingStep | null>(null);
  const [lines, setLines] = useState<{ who: string; text: string }[]>([]);
  const [input, setInput] = useState("");
  const [preview, setPreview] = useState<OnboardingPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<CapabilitiesReport | null>(null);
  const [folders, setFolders] = useState<FolderView[]>([]);
  // Выбранные по умолчанию: то, что уже работает. Пользователь снимает
  // галочки, а не ищет, где их поставить.
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [picked, setPicked] = useState<Set<string>>(new Set());
  // Весь диск или отдельные папки. Второе — узкий вариант, и он
  // стоит по умолчанию: широкий доступ человек выдаёт сам и осознанно.
  const [wholeDisk, setWholeDisk] = useState(false);
  // Режим выполнения и уровень доступа. Пустая строка означает
  // «ещё не выбрано», и это не то же самое, что выбор не сделан:
  // пока пусто, кнопка «дальше» заблокирована.
  const [режим, setРежим] = useState("");
  const [уровень, setУровень] = useState("");

  useEffect(() => {
    // `onboarding_current`, а не `onboarding_start`: первый читает
    // текущий шаг, второй начинает с нуля.
    //
    // Разница видна не сразу, но стоит целого знакомства: компонент
    // монтируется не только при первом открытии, но и при возврате на
    // вкладку, и при перерисовке окна. Знакомство слетало на первый
    // шаг, стоило переключиться куда угодно.
    invoke<OnboardingStep>("onboarding_current", { lang: язык() })
      .then(setStep)
      .catch((e) => setError(String(e)));
    invoke<CapabilitiesReport>("capabilities_report", { lang: язык() })
      .then((r) => {
        setReport(r);
        setChosen(new Set(r.rows.filter((x) => x.available).map((x) => x.id)));
      })
      .catch(() => undefined);
    invoke<FolderView[]>("onboarding_folders")
      .then(setFolders)
      .catch(() => undefined);
  }, []);

  // Итог подгружается по самому шагу, а не в том месте, где шаг
  // закончился.
  //
  // Раньше предпросмотр брали только из свободного ответа, и путь
  // через кнопки его вовсе не касался: человек доходил до последнего
  // шага и видел пустое поле вместо «что получится». Привязка к шагу
  // закрывает все дороги сразу — и кнопки, и текст, и возврат на
  // вкладку.
  useEffect(() => {
    if (!step || step.step !== "summary" || preview) return;
    invoke<OnboardingPreview>("onboarding_preview", { lang: язык() })
      .then(setPreview)
      .catch((e) => setError(String(e)));
  }, [step, preview]);

  const pickPaths = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<OnboardingSay>("onboarding_set_paths", { lang: язык(),
        paths: Array.from(picked),
      });
      setStep(r.next);
      setLines((prev) => [...prev, { who: "вы", text: `папок выбрано: ${picked.size}` }]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Переключатель «весь диск». Отправляется сразу: на шаге папок
  // это единственный доступный выбор, и ждать отдельной кнопки
  // «дальше» было бы лишним шагом без смысла.
  const toggleWholeDisk = async (want: boolean) => {
    setBusy(true);
    setError(null);
    setWholeDisk(want);
    try {
      const r = await invoke<OnboardingSay>("onboarding_set_whole_disk", { lang: язык(),
        enabled: want,
      });
      if (want) setStep(r.next);
    } catch (e) {
      setWholeDisk(!want);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Режим и уровень доступа отправляются одной командой: по отдельности
// шаг не двигался бы, потому что решение одно — насколько секретарь
// будет действовать сам.
const отправитьРежимы = async () => {
  setBusy(true);
  setError(null);
  try {
    const r = await invoke<OnboardingSay>("onboarding_set_modes", { lang: язык(),
      executor: режим,
      approval: уровень,
    });
    setStep(r.next);
    setLines((prev) => [...prev, { who: "ты", text: "уровни выбраны" }]);
  } catch (e) {
    setError(String(e));
  } finally {
    setBusy(false);
  }
};

const pickTools = async () => {
    setBusy(true);
    setError(null);
    try {
      const enable: string[] = [];
      const disable: string[] = [];
      report?.rows.forEach((r) => {
        if (chosen.has(r.id)) enable.push(r.id);
        else disable.push(r.id);
      });
      const r = await invoke<OnboardingSay>("onboarding_set_tools", { lang: язык(), enable, disable });
      setStep(r.next);
      setLines((prev) => [
        ...prev,
        {
          who: "вы",
          text: `отмечено инструментов: ${enable.length}, снято: ${disable.length}`,
        },
      ]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const say = async () => {
    const text = input.trim();
    if (!text || !step) return;
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<OnboardingSay>("onboarding_say", { text, lang: язык() });
      setLines((prev) => [...prev, { who: "вы", text }]);
      setInput("");
      if (!r.understood) {
        // Модель ответила не тем и не разобралось. Не злимся и не
        // угадываем: тот же вопрос ещё раз.
        setError("Не понял ответ. Попробуем тот же вопрос ещё раз.");
      }
      setStep(r.next);
      if (r.finished) {
        const p = await invoke<OnboardingPreview>("onboarding_preview", { lang: язык() });
        setPreview(p);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      // `onboarding_confirm`, а не `onboarding_apply`: применение
      // требует законченного знакомства, а закончить его должен
      // человек — этой кнопкой. Раньше здесь стояло применение, и
      // кнопка упиралась в проверку, которую сама же должна была
      // снимать. Знакомство нельзя было закончить, настройки не
      // сохранялись, и секретарь оставался без единой папки —
      // замечено 15.10.2026.
      const p = await invoke<OnboardingPreview>("onboarding_confirm", { lang: язык() });
      setPreview(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const restart = async () => {
    setBusy(true);
    setError(null);
    setLines([]);
    try {
      const начало = await invoke<OnboardingStep>("onboarding_start", { lang: язык() });
      // Порядок здесь и есть исправление. Раньше `setPreview(null)`
      // стояло до `await`, и в промежутке React успевал отрисовать
      // состояние «шаг summary, итога нет». Эффект загрузки итога
      // видел ровно это и запрашивал итог у Rust — где знакомство
      // ещё не было сброшено. Старые данные возвращались и
      // показывались на первом шаге: человек жал «начать заново»,
      // а вопросы и папки оставались прежними.
      //
      // Теперь сброс идёт первым, и пустого итога при старом шаге
      // не возникает: либо шаг ещё summary и итог на месте, либо
      // знакомство уже чистое.
      setPreview(null);
      setStep(начало);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (error && !step) {
    return (
      <div className="panel">
        <h2>{t("Знакомство")}</h2>
        <p className="error">{error}</p>
      </div>
    );
  }

  if (!step) {
    return (
      <div className="panel">
        <h2>{t("Знакомство")}</h2>
        <p className="muted">{t("Готовлюсь…")}</p>
      </div>
    );
  }

  return (
    <div className="panel">
      <h2>{t("Знакомство")}</h2>
      <p className="muted">
        Шаг {step.index} из {step.total}: {step.title}. Собираем {step.what}.
      </p>

      <div className="onb-log">
        {lines.length === 0 ? (
          <div className="onb-bubble onb-agent">
            Сейчас {step.what}. Напиши ответ обычной фразой — я сам пойму.
          </div>
        ) : (
          lines.map((l, i) => (
            <div key={i} className={`onb-bubble ${l.who === "вы" ? "onb-user" : "onb-agent"}`}>
              {l.text}
            </div>
          ))
        )}
      </div>

      {error && <p className="error">{error}</p>}

      {step.step === "tools" && report ? (
        <ToolPicker
          rows={report.rows}
          chosen={chosen}
          busy={busy}
          onToggle={(id, want) => {
            setChosen((prev) => {
              const next = new Set(prev);
              if (want) next.add(id);
              else next.delete(id);
              return next;
            });
          }}
          onDone={pickTools}
        />
      ) : step.step === "folders" ? (
        <>
          <p className="muted">{t("Реши, насколько широко секретарь сможет читать. На этом шаге содержимое не читается — только названия папок.")}

          </p>

          {/* Сначала широкий вариант, потом узкий. Порядок не
              случайный: человек сначала решает «насколько широко»,
              и лишь потом выбирает, где именно. Обратный порядок
              заставил бы его отмечать папки, чтобы потом узнать,
              что можно было просто включить один переключатель. */}
          <button
            type="button"
            className={wholeDisk ? "wide-opt on" : "wide-opt"}
            disabled={busy}
            aria-pressed={wholeDisk}
            onClick={() => toggleWholeDisk(!wholeDisk)}
          >
            <span className="wide-title">{t("читать весь диск")}</span>
            <span className="wide-note">{t("Секретарь сможет читать что угодно в любой папке. Удобно, если документы разбросаны. Он всё равно не напишет и не удалит ничего без твоего вопроса.")}


            </span>
          </button>

          {!wholeDisk && (
            <>
              <p className="muted alt-lead">{t("или отметь отдельные папки")}</p>
              <Picker
                items={folders.map((f) => ({ id: f.label, title: f.label, note: f.path }))}
                chosen={picked}
                onToggle={(id, want) => {
                  setPicked((prev) => {
                    const next = new Set(prev);
                    if (want) next.add(id);
                    else next.delete(id);
                    return next;
                  });
                }}
                onDone={pickPaths}
                busy={busy}
                empty={t("Папок по умолчанию не нашлось. Можно назвать свою в поле ниже.")}
              />
              {folders.length === 0 && (
                <div className="onb-input">
                  <input
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") say(); }}
                    placeholder={t("путь к папке")}
                    disabled={busy}
                  />
                  <button type="button" onClick={say} disabled={busy || !input.trim()}>{t("сказать")}
                  </button>
                </div>
              )}
            </>
          )}
        </>
      ) : step.step === "levels" ? (
        <>
          <p className="muted">{t("Два решения, и оба про то, насколько секретарь будет действовать сам. Сначала — показывать или делать.")}

          </p>

          <p className="muted alt-lead">{t("режим")}</p>
          <div className="picks">
            {РЕЖИМЫ.map((m) => (
              <button
                key={m.id}
                type="button"
                className={режим === m.id ? "pick pick-on" : "pick"}
                disabled={busy}
                onClick={() => setРежим(m.id)}
              >
                <span className="pick-box">{режим === m.id ? "×" : ""}</span>
                <span className="pick-body">
                  <span className="pick-name">{m.title}</span>
                  <span className="pick-what">{m.note}</span>
                </span>
              </button>
            ))}
          </div>

          <p className="muted alt-lead">{t("уровень доступа")}</p>
          <div className="picks">
            {УРОВНИ.map((u) => (
              <button
                key={u.id}
                type="button"
                className={уровень === u.id ? "pick pick-on" : "pick"}
                disabled={busy}
                onClick={() => setУровень(u.id)}
              >
                <span className="pick-box">{уровень === u.id ? "×" : ""}</span>
                <span className="pick-body">
                  <span className="pick-name">{u.title}</span>
                  <span className="pick-what">{u.note}</span>
                </span>
              </button>
            ))}
          </div>

          <div className="onb-input">
            <button
              type="button"
              onClick={отправитьРежимы}
              disabled={busy || !режим || !уровень}
            >{t("дальше")}
            </button>
          </div>
        </>
      ) : (
        <>
          {!preview && (
            <div className="onb-input">
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") say(); }}
                placeholder={t("твой ответ")}
                disabled={busy}
              />
              <button type="button" onClick={say} disabled={busy || !input.trim()}>{t("сказать")}
              </button>
            </div>
          )}
        </>
      )}

      {preview && (
        <div className="onb-preview">
          <h3>{t("Итог")}</h3>
          <p>{preview.summary}</p>

          <div className="cap-row cap-ok">
            <div className="cap-head"><b>{t("Как буду работать")}</b></div>
            <div className="cap-what">{preview.settings.executor.title}</div>
            <div className="cap-evidence">{preview.settings.executor.explain}</div>
            <div className="cap-what" style={{ marginTop: 6 }}>
              {preview.settings.approval_mode.title}
            </div>
            <div className="cap-evidence">{preview.settings.approval_mode.explain}</div>
          </div>

          {preview.settings.paths.length > 0 && (
            <div className="cap-row cap-ok">
              <div className="cap-head"><b>{t("Папки")}</b></div>
              <div className="cap-what">{preview.settings.paths.join(", ")}</div>
            </div>
          )}

          <p className="muted">{t("Ничего ещё не применено. Проверь и нажми «сохранить», если всё верно.")}
          </p>
          <div className="onb-input">
            <button type="button" onClick={apply} disabled={busy}>{t("сохранить")}</button>
            <button type="button" onClick={restart} disabled={busy}>{t("начать заново")}</button>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------

function Settings({ profile, onChange }: {
  profile: ProfileView;
  onChange: (p: ProfileView) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function call<T>(fn: () => Promise<T>) {
    setBusy(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel">
      <h2>{t("Ваш компьютер")}</h2>
      <dl className="facts">
        <div><dt>{t("Видеокарта")}</dt><dd>{profile.gpu_name}</dd></div>
        <div>
          <dt>{t("Видеопамять")}</dt>
          <dd>
            {gb(profile.vram_mb)}
            <span className="muted"> доступно {gb(profile.vram_usable_mb)}</span>
          </dd>
        </div>
        <div><dt>{t("Процессор")}</dt><dd>{profile.cpu_brand}</dd></div>
        <div><dt>{t("Оперативная память")}</dt><dd>{gb(profile.ram_mb)}</dd></div>
      </dl>

      <h2>{t("Профиль")}</h2>
      <p className="summary">{profile.summary}</p>
      {profile.unsupported_reason && (
        <p className="error">Не поддерживается: {profile.unsupported_reason}</p>
      )}

      <ul className="tiers">
        {profile.available_tiers.map((уровень) => {
          const active = уровень.tier === profile.effective;
          const blocked = уровень.blocked_because !== null;
          return (
            // Причина и пометка лежат внутри карточки, а не рядом с
            // ней. Раньше `<em>` был соседом пункта списка и вылезал
            // за его границы — на скриншоте это и читалось как наезд
            // текста на подпись кнопки.
            <li key={уровень.tier} className={active ? "tier active" : "tier"}>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  call(() =>
                    invoke<ProfileView>("set_profile_tier", {
                      tier: active && profile.forced ? null : уровень.tier,
                    })
                  ).then((p) => p && onChange(p))
                }
                title={уровень.blocked_because ?? t("Выбрать этот профиль")}
              >
                <strong>{уровень.tier}</strong>
                <span>{уровень.describe.replace(`${уровень.tier} — `, "")}</span>
              </button>
              {blocked && <p className="tier-note muted">{уровень.blocked_because}</p>}
              {active && !blocked && <p className="tier-note chosen">{t("выбран")}</p>}
            </li>
          );
        })}
      </ul>

      {profile.forced && (
        <button type="button" className="link" disabled={busy}
          onClick={() => call(() => invoke<ProfileView>("set_profile_tier", { tier: null }))
            .then((p) => p && onChange(p))}>{t("вернуть автоматическое определение")}
        </button>
      )}

      <h2>{t("Контекст")}</h2>
      <p className="muted">{t("Сколько токенов модель держит в голове за раз. Больше — больше помнит, но дольше думает: на 24 000 токенах первый ответ приходит через 11 секунд.")}


      </p>
      <div className="context-row">
        {[16384, 32768, 65536, 131072, 262144].map((c) => (
          <button key={c} type="button" disabled={busy}
            className={c === profile.context ? "chip active" : "chip"}
            onClick={() => call(() => invoke<ProfileView>("set_context", { context: c }))
              .then((p) => p && onChange(p))}>
            {c / 1024}K
          </button>
        ))}
      </div>

      {error && <p className="error">{error}</p>}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Чат (S2)
// ---------------------------------------------------------------------------

function Chat({ model, hidden }: { model: ModelStatus; hidden?: boolean }) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<AttachmentView[]>([]);
  const bottom = useRef<HTMLDivElement>(null);
  const requestId = useRef(0);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [
      listen<{ request_id: number; delta: string }>("chat:chunk", (e) => {
        if (e.payload.request_id !== requestId.current) return;
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") {
            next[next.length - 1] = { ...last, content: last.content + e.payload.delta };
          }
          return next;
        });
      }),
      listen<{
        request_id: number; outcome: string; prompt_tokens: number | null;
        completion_tokens: number | null; tok_s: number | null;
        error: string | null; tool_calls: unknown;
      }>("chat:done", (e) => {
        if (e.payload.request_id !== requestId.current) return;
        setBusy(false);
        const p = e.payload;
        if (p.error) {
          setNotice(p.error);
        } else {
          const bits: string[] = [];
          if (p.prompt_tokens) bits.push(`${p.prompt_tokens} в промпте`);
          if (p.tok_s) bits.push(`${p.tok_s.toFixed(1)} tok/s`);
          if (p.outcome === "stop") bits.push("остановлено");
          setNotice(bits.join(" · ") || null);
        }
      }),
    ];
    return () => { unlisteners.forEach((p) => p.then((f) => f())); };
  }, []);

  useEffect(() => {
    // Пока вкладка скрыта, прокрутка бессмысленна; при возврате
    // прокручиваем к последней реплике — иначе человек видит
    // начало переписки вместо её конца.
    if (hidden) return;
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, hidden]);

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || busy || !model.running) return;

    const next: Message[] = [...messages, { role: "user", content: text }];
    setMessages([...next, { role: "assistant", content: "" }]);
    setInput("");
    setBusy(true);
    setNotice(null);

    try {
      // Номер задаём мы, до отправки. Команда возвращает его уже
      // после того, как весь ответ прочитан и события отправлены,
      // так что взять номер из ответа нельзя: события пришли бы
      // раньше, чем мы его записали, и все отбросились бы проверкой
      // «это мой запрос?». Ответ не появлялся никогда.
      requestId.current += 1;
      const мой = requestId.current;
      await invoke<number>("chat", {
        requestId: мой,
         messages: next.map(({ role, content }, index) => ({
           role,
           content,
           ...(index === next.length - 1 && attachments.length > 0
             ? { attachments: attachments.map((attachment) => attachment.id) }
             : {}),
         })),
        maxTokens: 1024,
      });
    } catch (e) {
      setBusy(false);
      setNotice(String(e));
      setMessages(next);
    }
  }, [attachments, busy, input, messages, model.running]);

  async function attach(files: FileList | null) {
    if (!files) return;
    try {
      for (const file of Array.from(files)) {
        const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
        const attachment = await invoke<AttachmentView>("attachment_add", {
          name: file.name,
          mime: file.type || "application/octet-stream",
          data: bytes,
        });
        setAttachments((current) => [...current, attachment]);
      }
    } catch (error) {
      setNotice(String(error));
    }
  }

  return (
    <section className="chat chat-panel" style={hidden ? { display: "none" } : undefined}>
      <div className="cyber-window-topline" aria-hidden="true">[ ARKHA // MODEL CHAT ] ---- CONTEXT LINK ---- [ ONLINE ]</div>
      <h2>Чат с моделью</h2>
      <div className="messages">
        {messages.length === 0 && (
          <p className="muted">
            Модель {model.recipe ?? "не выбрана"} готова. Напишите что-нибудь.
          </p>
        )}
        {messages.map((m, i) => (
          <div key={i} className={`msg ${m.role}`}>{m.content || "…"}</div>
        ))}
        <div ref={bottom} />
      </div>

      {busy && (
        <button type="button" className="stop" onClick={() => invoke("stop_generation")}>{t("остановить ответ")}
        </button>
      )}
      {notice && <p className="muted notice">{notice}</p>}

      <form className="composer" onSubmit={(e) => { e.preventDefault(); send(); }}>
        <label className="attach-button" title={t("Прикрепить файл")}>
          +
          <input type="file" hidden multiple onChange={(event) => void attach(event.target.files)} />
        </label>
        <input
          className="app-input"
          value={input}
          onChange={(e) => setInput(e.currentTarget.value)}
          placeholder={model.running ? "Напишите сообщение" : "Сначала запустите модель"}
          disabled={!model.running || busy}
          aria-label={t("Сообщение")}
        />
        <button className="app-send" type="submit" disabled={!input.trim() || busy || !model.running}>{t("Отправить")}
        </button>
      </form>
      {attachments.length > 0 && (
        <div className="chat-attachments">
          {attachments.map((attachment) => (
            <button type="button" key={attachment.id} onClick={() => setAttachments((current) => current.filter((item) => item.id !== attachment.id))}>
              {attachment.name} ×
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Модель
// ---------------------------------------------------------------------------

interface Problem {
  what: string;
  why: string;
  how: string;
}

interface Preflight {
  problems: Problem[];
  usable_recipe: string | null;
  models_dir: string;
}

interface RemoteFile {
  path: string;
  size: number;
  sha256: string;
}

type DownloadState =
  | { kind: "idle" }
  | { kind: "meta" }
  | { kind: "busy"; downloaded: number; total: number; mib: number }
  | { kind: "verifying"; size: number }
  | { kind: "done"; path: string }
  | { kind: "error"; message: string };

function gib(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(2)} ГиБ`;
}

/// Что приложение знает о движке. Имена полей латиницей — их
/// отдаёт Rust, и переводить кириллицу на стороне TypeScript
/// незачем.
interface EngineState {
  found: boolean;
  path: string | null;
  searched: string[];
  cudaNeeded: boolean;
  cudaPresent: boolean;
  gpuName: string;
}

interface RuntimePackage {
  id: string;
  name: string;
  engine: string;
  backend: string;
  version: string;
  path: string;
  executable: string | null;
  installed: boolean;
  active: boolean;
  compatible: boolean;
  status: string;
  notes: string[];
  managed: boolean;
}

interface RuntimeCompatibility {
  runtimeId: string;
  compatible: boolean;
  errors: string[];
  warnings: string[];
  checkedFiles: string[];
}

interface RuntimeManifest {
  id: string;
  name: string;
  engine: string;
  backend: string;
  version: string;
  executable: string | null;
  required_files: string[];
  requires_cuda: boolean;
}

interface LocalModel {
  name: string;
  path: string;
  sizeBytes: number;
  active: boolean;
  supported: boolean;
}

function modelFamily(name: string): string {
  const stem = name.replace(/\.gguf$/i, "");
  const quant = stem.search(/(?:IQ\d+_[A-Z0-9_]+|Q\d+(?:_[A-Z0-9_]+)?|BF16|FP16|FP32|F16|F32)/i);
  if (quant <= 0) return stem;
  return stem.slice(0, quant).replace(/[-_.]+$/, "") || stem;
}

function quantization(name: string): string {
  const stem = name.replace(/\.gguf$/i, "");
  const match = stem.match(/(?:IQ\d+_[A-Z0-9_]+|Q\d+(?:_[A-Z0-9_]+)?|BF16|FP16|FP32|F16|F32)/i);
  return match?.[0] ?? "other";
}

interface SkillRegistryItem {
  id: string;
  name: string;
  version: string;
  description: string;
  source: string;
  path: string;
  tools: string[];
  permissions: string[];
  enabled: boolean;
  manifest: boolean;
}

type EngineProgress =
  | { state: "ищем"; assets: string[] }
  | {
      state: "качаем";
      файл: string;
      downloaded: number;
      total: number;
      megabytes_per_s: number;
    }
  | { state: "распаковка"; файл: string; куда: string }
  | { state: "готово"; в_какую_папку: string };

/// Панель движка: что нашлось и чего не хватает.
///
/// Показывается всегда, а не только когда что-то сломалось.
/// Человек должен видеть, что программа работает с процессором и
/// картой не пользуется, пока он сам этого не попросил.
function EnginePanel() {
  const [state, setState] = useState<EngineState | null>(null);
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [ошибка, setОшибка] = useState<string | null>(null);

  const опросить = () =>
    invoke<EngineState>("engine_status").then(setState).catch(() => undefined);

  useEffect(() => {
    опросить();
  }, []);

  useEffect(() => {
    const un = listen<EngineProgress>("engine://progress", (e) => {
      setProgress(e.payload);
      if (e.payload.state === "готово") опросить();
    });
    return () => { un.then((f) => f()); };
  }, []);

  const качать = async () => {
    setОшибка(null);
    setProgress({ state: "ищем", assets: [] });
    try {
      const итог = await invoke<EngineState>("engine_download_cuda");
      setState(итог);
      setProgress(null);
    } catch (e) {
      setProgress(null);
      setОшибка(String(e));
    }
  };

  if (!state) return null;

  const идёт = progress !== null;

  return (
    <section className="panel">
      <h2>{t("Движок")}</h2>

      <p className="muted">
        {state.found
          ? t("Движок найден. Работает на процессоре.")
          : t("Движок не найден ни рядом с программой, ни в папке проекта. Модель не запустится.")}
      </p>

      {state.gpuName && (
        <p className="muted">{t("Видеокарта: ")}{state.gpuName}</p>
      )}

      {state.cudaPresent ? (
        <p className="muted">{t("Библиотеки карты на месте — движок работает на видеокарте.")}</p>
      ) : state.cudaNeeded ? (
        <>
          <p>
            {t("Видеокарта есть, а библиотек для неё нет: ")}
            <strong>523 МиБ</strong>
            {t(". Без них агент пойдёт на процессоре, медленно. Скачивание делаете вы, и только один раз.")}
          </p>
          <button onClick={качать} disabled={идёт}>
            {идёт ? t("Идёт загрузка…") : t("Скачать библиотеки карты")}
          </button>
        </>
      ) : (
        <p className="muted">
          {t("Библиотеки карты не нужны: видеокарты NVIDIA на этой машине нет. Модель работает на процессоре.")}
        </p>
      )}

      {progress && progress.state === "качаем" && (
        <p className="muted">
          {t("Качаем: ")}
          {(progress.downloaded / 1048576).toFixed(0)}
          {" / "}
          {(progress.total / 1048576).toFixed(0)}
          {" МиБ, "}
          {progress.megabytes_per_s.toFixed(1)}
          {" МиБ/с"}
        </p>
      )}
      {progress && progress.state === "распаковка" && (
        <p className="muted">{t("Распаковываем: ")}{progress.файл}</p>
      )}

      {ошибка && <p className="muted">{ошибка}</p>}

      {!state.found && (
        <p className="muted">
          {t("Искали здесь:")}
          <br />
          {state.searched.map((p) => (
            <span key={p}>{p}</span>
          ))}
        </p>
      )}
    </section>
  );
}

function RuntimePanel() {
  const [packages, setPackages] = useState<RuntimePackage[] | null>(null);
  const [compatibility, setCompatibility] = useState<RuntimeCompatibility[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [install, setInstall] = useState({
    url: "",
    sha256: "",
    id: "",
    name: "",
    engine: "llama.cpp",
    backend: "cuda",
    version: "",
    executable: "llama/llama-server.exe",
    requiredFiles: "",
    requiresCuda: false,
  });

  const refresh = () => {
    invoke<RuntimePackage[]>("runtime_list")
      .then((result) => {
        setPackages(result);
        void invoke<RuntimeCompatibility[]>("runtime_check").then(setCompatibility).catch(() => undefined);
        setError(null);
      })
      .catch((reason) => setError(String(reason)));
  };

  const activate = async (id: string) => {
    try {
      const result = await invoke<RuntimePackage[]>("runtime_set_active", { id });
      setPackages(result);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const installRuntime = async () => {
    setInstalling(true);
    setError(null);
    try {
      const manifest: RuntimeManifest = {
        id: install.id,
        name: install.name,
        engine: install.engine,
        backend: install.backend,
        version: install.version,
        executable: install.executable || null,
        required_files: install.requiredFiles
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
        requires_cuda: install.requiresCuda,
      };
      await invoke<RuntimePackage[]>("runtime_install", {
        url: install.url,
        sha256: install.sha256,
        manifest,
      });
      setInstall((current) => ({ ...current, url: "", sha256: "" }));
      refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setInstalling(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  return (
    <section className="panel runtime-panel">
      <div className="runtime-panel-heading">
        <div>
          <h2>{t("Runtime")}</h2>
          <p className="muted">{t("Установленные движки и версии")}</p>
          <p className="muted">{t("Runtime-файлы общие для аккаунтов: установка добавляет версию, существующие версии не удаляются и не перезаписываются.")}</p>
        </div>
        <button type="button" className="chip" onClick={refresh}>
          {t("Обновить")}
        </button>
      </div>

      {error && <p className="error">{error}</p>}
      {!packages && !error && <p className="muted">{t("Проверяем runtime…")}</p>}
      {packages && packages.length === 0 && (
        <p className="muted">{t("Установленных runtime пока нет")}</p>
      )}
      <div className="runtime-list">
        {packages?.map((runtime) => (
          <article className="runtime-card" key={runtime.id}>
            <div className="runtime-card-main">
              <div>
                <strong>{runtime.name}</strong>
                <span className="runtime-meta">
                  {runtime.engine} · {runtime.backend} · {runtime.version}
                </span>
              </div>
              <span className={`runtime-status ${runtime.compatible ? "ok" : "bad"}`}>
                {runtime.active ? t("активен") : runtime.status}
              </span>
            </div>
            <div className="runtime-path" title={runtime.path}>{runtime.path}</div>
            {runtime.notes.map((note) => (
              <div className="runtime-note" key={note}>{note}</div>
            ))}
            {compatibility.find((report) => report.runtimeId === runtime.id)?.errors.map((error) => (
              <div className="runtime-error" key={error}>{error}</div>
            ))}
            <div className="runtime-actions">
              {runtime.managed && (
                <>
                  <button type="button" className="chip" onClick={() => void invoke("runtime_reveal", { id: runtime.id })}>
                    {t("Открыть папку")}
                  </button>
                </>
              )}
            </div>
            {runtime.installed && runtime.compatible && !runtime.active && (
              <button type="button" className="chip runtime-activate" onClick={() => void activate(runtime.id)}>
                {t("Сделать активным")}
              </button>
            )}
          </article>
        ))}
      </div>

      <details className="runtime-install">
        <summary>{t("Установить runtime из ZIP")}</summary>
        <p className="muted">{t("Нужны HTTPS-ссылка, SHA-256 и manifest-параметры пакета.")}</p>
        <div className="runtime-install-grid">
          {([[
            "url", "HTTPS URL"
          ], [
            "sha256", "SHA-256"
          ], [
            "id", "ID"
          ], [
            "name", "Название"
          ], [
            "version", "Версия"
          ], [
            "executable", "Executable внутри ZIP"
          ], [
            "requiredFiles", "Обязательные файлы через запятую"
          ]] as const).map(([field, label]) => (
            <label key={field}>
              <span>{label}</span>
              <input
                value={install[field]}
                onChange={(event) => setInstall((current) => ({ ...current, [field]: event.target.value }))}
              />
            </label>
          ))}
          <label>
            <span>{t("Backend")}</span>
            <select value={install.backend} onChange={(event) => setInstall((current) => ({ ...current, backend: event.target.value }))}>
              <option value="cpu">CPU</option>
              <option value="cuda">CUDA</option>
              <option value="vulkan">Vulkan</option>
            </select>
          </label>
          <label className="runtime-checkbox">
            <input type="checkbox" checked={install.requiresCuda} onChange={(event) => setInstall((current) => ({ ...current, requiresCuda: event.target.checked }))} />
            <span>{t("Требуется CUDA")}</span>
          </label>
        </div>
        <button type="button" onClick={() => void installRuntime()} disabled={installing}>
          {installing ? t("Установка…") : t("Установить и проверить")}
        </button>
      </details>
    </section>
  );
}

function ModelLibrary() {
  const [models, setModels] = useState<LocalModel[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    invoke<LocalModel[]>("models_list")
      .then(setModels)
      .catch((reason) => setError(String(reason)));
  };

  useEffect(() => { refresh(); }, []);

  const select = async (name: string) => {
    try {
      setModels(await invoke<LocalModel[]>("model_set_active", { name }));
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const groups = Array.from(
    (models ?? []).reduce((result, model) => {
      const family = modelFamily(model.name);
      const group = result.get(family) ?? [];
      group.push(model);
      result.set(family, group);
      return result;
    }, new Map<string, LocalModel[]>()),
  ).sort(([left], [right]) => left.localeCompare(right));

  return (
    <section className="panel model-library">
      <div className="runtime-panel-heading">
        <div>
          <h2>{t("Модели")}</h2>
          <p className="muted">{t("Локальная библиотека GGUF")}</p>
        </div>
        <button type="button" className="chip" onClick={refresh}>{t("Обновить")}</button>
      </div>
      {error && <p className="error">{error}</p>}
      {!models && !error && <p className="muted">{t("Сканируем папку моделей…")}</p>}
      {models && models.length === 0 && <p className="muted">{t("GGUF-моделей пока нет")}</p>}
      <div className="model-library-list">
        {groups.map(([family, variants]) => {
          const active = variants.find((model) => model.active) ?? variants[0];
          return (
            <article className="model-family-card" key={family}>
              <div className="model-family-heading">
                <div>
                  <strong>{family}</strong>
                  <span className="runtime-meta">{variants.length} {t("вариантов квантования")}</span>
                </div>
                <select
                  className="model-quant-select"
                  value={active.name}
                  onChange={(event) => void select(event.target.value)}
                >
                  {variants.map((variant) => (
                    <option key={variant.name} value={variant.name} disabled={!variant.supported}>
                      {quantization(variant.name)} · {gib(variant.sizeBytes)}{variant.supported ? "" : ` · ${t("нет рецепта")}`}
                    </option>
                  ))}
                </select>
              </div>
              <div className="runtime-path" title={active.path}>{active.path}</div>
              <div className="model-family-footer">
                <span className="runtime-meta">{gib(active.sizeBytes)} · {active.supported ? t("рецепт найден") : t("нет рецепта профиля")}</span>
                <button
                  type="button"
                  className={`model-select ${active.active ? "selected" : ""}`}
                  disabled={active.active || !active.supported}
                  onClick={() => void select(active.name)}
                >
                  {active.active ? t("выбрана") : t("выбрать")}
                </button>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function SkillsPanel() {
  const [skills, setSkills] = useState<SkillRegistryItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [searchInput, setSearchInput] = useState("");
  const [query, setQuery] = useState("");
  const [viewing, setViewing] = useState<{ name: string; content: string } | null>(null);

  const refresh = () => {
    invoke<SkillRegistryItem[]>("skills_list")
      .then(setSkills)
      .catch((reason) => setError(String(reason)));
  };

  useEffect(() => { refresh(); }, []);

  const toggle = async (skill: SkillRegistryItem) => {
    if (!skill.enabled && skill.permissions.length > 0) {
      const permissions = skill.permissions.join(", ");
      const confirmed = window.confirm(
        `${t("Навык запрашивает разрешения")}: ${permissions}\n\n${t("Разрешения не выдаются автоматически и будут проверяться при каждом вызове инструмента.")}\n\n${t("Включить навык?")}`,
      );
      if (!confirmed) return;
    }
    try {
      const next = await invoke<SkillRegistryItem[]>(skill.enabled ? "skills_disable" : "skills_enable", { id: skill.id });
      setSkills(next);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const view = async (skill: SkillRegistryItem) => {
    try {
      setViewing({ name: skill.name, content: await invoke<string>("skills_read", { id: skill.id }) });
    } catch (reason) {
      setError(String(reason));
    }
  };

  const filtered = skills?.filter((skill) =>
    `${skill.name} ${skill.description} ${skill.permissions.join(" ")}`.toLowerCase().includes(query.toLowerCase()),
  );

  return (
    <section className="panel skills-panel">
      <div className="runtime-panel-heading">
        <div>
          <h2>{t("Навыки")}</h2>
          <p className="muted">{t("Процедуры, доступные агенту")}</p>
        </div>
        <button type="button" className="chip" onClick={refresh}>{t("Обновить")}</button>
      </div>
      <form className="skills-search-row" onSubmit={(event) => { event.preventDefault(); setQuery(searchInput.trim()); }}>
        <input className="skills-search" value={searchInput} onChange={(event) => setSearchInput(event.target.value)} placeholder={t("Поиск навыков")} />
        <button type="submit" className="chip">{t("Найти")}</button>
      </form>
      {error && <p className="error">{error}</p>}
      {!skills && !error && <p className="muted">{t("Проверяем навыки…")}</p>}
      <div className="skills-registry-list">
        {filtered?.map((skill) => (
          <article className="skill-registry-card" key={skill.id}>
            <div>
              <strong>{skill.name}</strong>
              <span className="runtime-meta">{skill.version} · {skill.source}</span>
              <p className="muted">{skill.description}</p>
              {skill.permissions.length > 0 && <small>{skill.permissions.join(", ")}</small>}
            </div>
            <div className="skill-actions">
              <button type="button" className="chip" onClick={() => void view(skill)}>{t("Просмотр")}</button>
              <button type="button" className={`skill-toggle ${skill.enabled ? "enabled" : ""}`} onClick={() => void toggle(skill)}>
                {skill.enabled ? t("включён") : t("выключен")}
              </button>
            </div>
          </article>
        ))}
      </div>
      {skills && filtered?.length === 0 && <p className="muted skills-empty">{t("Навыки не найдены")}</p>}
      {viewing && (
        <div className="skill-viewer">
          <div className="runtime-panel-heading"><h3>{viewing.name}</h3><button type="button" className="chip" onClick={() => setViewing(null)}>×</button></div>
          <pre>{viewing.content}</pre>
        </div>
      )}
    </section>
  );
}

function Downloader({ model }: { model: ModelStatus }) {
  const [repo, setRepo] = useState("unsloth/Qwen3.5-9B-GGUF");
  const [files, setFiles] = useState<RemoteFile[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [state, setState] = useState<DownloadState>({ kind: "idle" });
  const [hosts, setHosts] = useState<string[] | null>(null);
  // Подсказки по семейству. null означает «ещё не искали», пустой
  // массив — «искали, ничего не нашлось». Различать надо: при null
  // список скрывается, при пустом показывается подпись.
  const [hits, setHits] = useState<ModelHit[] | null>(null);
  const [ищет, setИщет] = useState(false);

  useEffect(() => {
    invoke<string[]>("download_hosts")
      .then(setHosts)
      .catch(() => undefined);
  }, []);

  // Rust присылает Progress с полем `state` (looking/downloading/verifying/done),
  // а интерфейс ждёт DownloadState с полем kind (meta/busy/verifying/done).
  // Преобразуем, иначе ProgressBar никогда не покажется.
  function из_progress(p: Record<string, unknown>): DownloadState {
    const стейт = (p as { state?: string }).state;
    const чис = (v: unknown) => (typeof v === "number" ? v : 0);
    const стр = (v: unknown) => (typeof v === "string" ? v : "");
    switch (стейт) {
      case "downloading":
        return {
          kind: "busy",
          downloaded: чис(p.downloaded),
          total: чис(p.total),
          mib: чис(p.mib_per_s),
        };
      case "looking":
        return { kind: "meta" };
      case "verifying":
        return { kind: "verifying", size: чис(p.size) };
      case "done":
        return { kind: "done", path: стр(p.path) };
      default:
        return { kind: "idle" };
    }
  }

  useEffect(() => {
    const un = listen<Record<string, unknown>>("download:progress", (e) => {
      setState(из_progress(e.payload));
    });
    return () => { un.then((f) => f()); };
  }, []);

  // Поиск с паузой. Без паузы запрос уходит на каждый символ:
  // это заметно и по сети, и по тому, что список дёргается.
  useEffect(() => {
    const запрос = repo.trim();
    if (запрос.length < 2) {
      setHits(null);
      return;
    }
    setИщет(true);
    const таймер = setTimeout(() => {
      invoke<ModelHit[]>("search_models", { query: запрос })
        .then(setHits)
        .catch(() => setHits([]))
        .finally(() => setИщет(false));
    }, 350);
    // Гонка: человек печатает быстро, и ответ на «gem» может
    // прийти после ответа на «gemma». Ответ на устаревший запрос
    // отбрасывается, иначе список дёргался бы назад.
    return () => clearTimeout(таймер);
  }, [repo]);

  async function loadFiles() {
    setState({ kind: "meta" });
    try {
      const list = await invoke<RemoteFile[]>("repo_files", { repo });
      setFiles(list.filter((f) => f.path.endsWith(".gguf")));
      setState({ kind: "idle" });
    } catch (e) {
      setFiles(null);
      setState({ kind: "error", message: String(e) });
    }
  }

  async function start() {
    if (!picked) return;
    setState({ kind: "meta" });
    try {
      await invoke<string>("download_model", { repo, file: picked });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>{t("Скачать модель")}</h2>

      {model.running ? (
        <p className="muted">
          Сейчас работает «{model.recipe}». Скачивание новой модели
          ничего не сломает, но запускать её придётся заново.
        </p>
      ) : (
        <p className="muted">{t("Под текущий профиль нужен квант из списка. Показан размер файла, а не скорость: она станет известна после сверки сводки.")}

        </p>
      )}

      <div className="repo-row">
        <input
          className="app-input"
          value={repo}
          onChange={(e) => setRepo(e.currentTarget.value)}
          placeholder={t("начни набирать: gemma, qwen, llama…")}
          aria-label={t("Репозиторий")}
        />
        <button className="chip" onClick={loadFiles}
          disabled={state.kind === "meta" || !repo.trim()}>{t("показать файлы")}
        </button>
      </div>

      {/* Подсказки по семейству. Список живёт под полем и исчезает
          сразу после выбора: оставшаяся строка подсказки закрывала бы
          то, ради чего человек в неё ткнул. */}
      {hits && hits.length > 0 && (
        <ul className="hints">
          {hits.map((h) => (
            <li key={h.repo}>
              <button
                type="button"
                onClick={() => { setRepo(h.repo); setHits(null); }}
              >
                <span className="hint-family">{h.family}</span>
                <span className="hint-repo">{h.repo}</span>
                <span className="hint-dl">
                  {h.downloads > 999 ? `${(h.downloads / 1000).toFixed(0)}k` : h.downloads}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {ищет && <p className="muted hint-status">{t("ищем…")}</p>}
      {hits && hits.length === 0 && !ищет && (
        <p className="muted hint-status">{t("по этому слову моделей не нашлось. Проверь написание или укажи репозиторий целиком.")}

        </p>
      )}

      {files && files.length > 0 && (
        <ul className="files">
          {files.map((f) => (
            <li key={f.path}>
              <button
                type="button"
                className={picked === f.path ? "file active" : "file"}
                onClick={() => setPicked(f.path)}
              >
                <span className="file-name">
                  {f.path.split("/").pop()}
                </span>
                <span className="muted">{gib(f.size)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {state.kind === "busy" && (
        <div className="progress">
          <div className="progress-bar">
            <div
              className="progress-fill"
              style={{ width: `${(state.downloaded / state.total) * 100}%` }}
            />
          </div>
          <p className="muted">
            {gib(state.downloaded)} из {gib(state.total)}, {state.mib.toFixed(1)} МБ/с
          </p>
          <button className="stop" onClick={() => invoke("cancel_download")}>{t("отменить")}
          </button>
        </div>
      )}

      {state.kind === "verifying" && (
        <p className="muted">Проверяем сводку файла, {gib(state.size)}…</p>
      )}

      {state.kind === "done" && (
        <p className="ok">Готово: {state.path}</p>
      )}

      {state.kind === "error" && (
        <p className="error">{state.message}</p>
      )}

      {picked && (
        <button className="app-send" onClick={start}
          disabled={state.kind === "busy" || state.kind === "meta"}>{t("Скачать")}
        </button>
      )}

      {hosts && (
        <details className="hosts">
          <summary className="muted">{t("если не скачивается: нужны разрешённые домены")}
          </summary>
          <ul>
            {hosts.map((h) => <li key={h}>{h}</li>)}
          </ul>
        </details>
      )}
    </section>
  );
}

function Problems({ pre }: { pre: Preflight }) {
  return (
    <section className="panel">
      <h2>{t("Что мешает запуску")}</h2>
      <ol className="problems">
        {pre.problems.map((p, i) => (
          <li key={i}>
            <strong>{p.what}</strong>
            <p className="muted">{p.why}</p>
            <p className="how">{p.how}</p>
          </li>
        ))}
      </ol>
      <p className="muted small">папка моделей: {pre.models_dir}</p>
    </section>
  );
}

function ModelBar({ model, onChange }: { model: ModelStatus; onChange: (m: ModelStatus) => void }) {
  const [starting, setStarting] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [showLog, setShowLog] = useState(false);
  const [pre, setPre] = useState<Preflight | null>(null);

  async function start() {
    setStarting(true);
    try {
      onChange(await invoke<ModelStatus>("start_model"));
    } catch (e) {
      onChange({ ...model, running: false, error: String(e) });
      setPre(await invoke<Preflight>("preflight_model").catch(() => null));
    } finally {
      setStarting(false);
    }
  }

  async function check() {
    setPre(await invoke<Preflight>("preflight_model").catch(() => null));
  }

  useEffect(() => {
    if (model.error) check();
  }, [model.error]);

  async function toggleLog() {
    const next = !showLog;
    setShowLog(next);
    if (next) {
      setLog(await invoke<string[]>("model_log", { lines: 60 }));
    }
  }

  return (
    <section className="panel model-panel">
      <h2>{t("Модель")}</h2>

      {!model.running ? (
        <button type="button" className="app-send" disabled={starting} onClick={start}>
          {starting ? "Проверяем и запускаем…" : "Запустить модель"}
        </button>
      ) : (
        <>
          <dl className="facts">
            <div><dt>{t("Рецепт")}</dt><dd>{model.recipe}</dd></div>
            <div><dt>{t("KV-кэш")}</dt><dd>{model.kv_cache}</dd></div>
            <div><dt>{t("Контекст")}</dt><dd>{model.context ? `${model.context / 1024}K` : "—"}</dd></div>
            <div>
              <dt>{t("Размещение")}</dt>
              <dd className={model.full_gpu ? "ok" : "bad"}>
                {model.full_gpu
                  ? `все слои на GPU (${model.gpu_layers}/${model.total_layers})`
                  : `частично: ${model.gpu_layers}/${model.total_layers}`}
              </dd>
            </div>
            <div><dt>{t("Устройство")}</dt><dd>{model.device ?? "—"}</dd></div>
            <div>
              <dt>{t("Скорость")}</dt>
              <dd>
                {model.tok_s ? `${model.tok_s} tok/s` : "не измерена"}
                {!model.verified && <span className="muted">{t("· рецепт не замерян")}</span>}
              </dd>
            </div>
            <div><dt>{t("Рассуждение")}</dt><dd>t=0.3 · top-p=0.9 · max=1024</dd></div>
          </dl>

          <div className="context-row">
            <button type="button" className="chip" onClick={() => invoke("stop_model")}>{t("остановить модель")}
            </button>
            <button type="button" className="chip" onClick={toggleLog}>
              {showLog ? "скрыть журнал" : "журнал запуска"}
            </button>
          </div>
        </>
      )}

      {model.error && <p className="error">{model.error}</p>}
      {pre && <Problems pre={pre} />}

      {showLog && (
        <pre className="log">{log.join("\n") || "журнал пуст"}</pre>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Приложение
// ---------------------------------------------------------------------------

type Tab = "chat" | "agent" | "download" | "settings" | "skills" | "capabilities" | "dashboard";

/**
 * Кегль интерфейса.
 *
 * Размер шрифта хранится в самом интерфейсе, а не в настройках
 * секретаря: это свойство окна, а не программы, и человек меняет
 * его, когда глаза устали, а не когда перенастраивает доступ.
 * Значение кладётся в CSS-переменную на корне, поэтому меняется
 * ровно то, что нарисовано, без перезагрузки и без перезапуска.
 */
const РАЗМЕРЫ_ШРИФТА = [13, 14, 15, 16, 18] as const;

/**
 * Варианты для шага «Уровни доступа».
 *
 * Идентификаторы совпадают с тем, что ждёт Rust: `dry-run`/`live`
 * и `actions`/`all`/`never`. Название и пояснение живут здесь, а не
 * приходят с сервера: на этом шаге человек принимает решение о том,
 * насколько секретарь будет действовать сам, и читать его должно
 * прямо на кнопке, а не после запроса.
 */
const РЕЖИМЫ = [
  {
    id: "dry-run",
    title: "только показывать",
    note: "Секретарь пишет, что собирается сделать, и ждёт. Ничего не происходит без твоего слова.",
  },
  {
    id: "live",
    title: "выполнять",
    note: "Согласные действия выполняются сразу. Опасные — всё равно спросит.",
  },
];

const УРОВНИ = [
  {
    id: "actions",
    title: "опасные — спросить",
    note: "Удаление, запуск программ и переписывание файлов требуют твоего подтверждения.",
  },
  {
    id: "all",
    title: "спросить всё",
    note: "Подтверждения на каждое действие. Медленнее, зато предсказуемо.",
  },
  {
    id: "never",
    title: "не спрашивать",
    note: "Делает всё, что разрешено. Опасно: промах в команде он не остановит.",
  },
];

interface VoiceSettings {
  enabled: boolean;
  auto_load: boolean;
  tts_enabled: boolean;
  language: string;
  voice: string | null;
  speed: number;
}

interface AppSettings {
  schema_version: number;
  language: "ru" | "en";
  font_size: number;
  active_model: string | null;
  active_runtime: string | null;
  voice: VoiceSettings;
  enabled_skills: string[];
  enabled_tools: string[];
  agent_mode: "plan" | "think" | "build";
  fallback_policy: string;
  user: {
    login: string;
    display_name: string;
  };
}

function LocalUserProfile({ settings, onSave }: { settings: AppSettings | null; onSave: (next: AppSettings) => void }) {
  const [login, setLogin] = useState("");
  const [displayName, setDisplayName] = useState("");

  useEffect(() => {
    setLogin(settings?.user.login ?? "");
    setDisplayName(settings?.user.display_name ?? "");
  }, [settings]);

  if (!settings) return null;
  const save = () => onSave({ ...settings, user: { login: login.trim(), display_name: displayName.trim() } });

  return (
    <section className="panel local-user-profile">
      <h2>{t("Локальный пользователь")}</h2>
      <p className="muted">{t("Профиль сохраняется локально и используется в новых чатах и запросах агента.")}</p>
      <div className="local-profile-grid">
        <label><span>login</span><input value={login} onChange={(event) => setLogin(event.target.value)} autoComplete="username" /></label>
        <label><span>{t("Имя")}</span><input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label>
      </div>
      <p className="muted">{t("Пароль появится вместе с локальным входом в будущем и пока не хранится.")}</p>
      <button type="button" onClick={save}>{t("Сохранить профиль")}</button>
    </section>
  );
}

function РазмерШрифта({ кегль, onChange }: { кегль: number; onChange: (value: number) => void }) {

  useEffect(() => {
    document.documentElement.style.setProperty("--кегль", `${кегль}px`);
  }, [кегль]);

  const шаг = РАЗМЕРЫ_ШРИФТА.indexOf(кегль as (typeof РАЗМЕРЫ_ШРИФТА)[number]);

  return (
    <section className="panel">
      <h2>{t("настройки")}</h2>

      <div className="pref-row">
        <div className="pref-text">
          <b>{t("Размер шрифта")}</b>
          <span className="muted">{t("Меняет весь интерфейс сразу. Если текст мелкий — поставь побольше, секретарь от этого не тупеет.")}

          </span>
        </div>
        <div className="stepper">
          <button
            type="button"
            aria-label={t("мельче")}
            disabled={шаг <= 0}
            onClick={() => onChange(РАЗМЕРЫ_ШРИФТА[шаг - 1] ?? кегль)}
          >
            A−
          </button>
          <span className="stepper-val">{кегль}</span>
          <button
            type="button"
            aria-label={t("крупнее")}
            disabled={шаг >= РАЗМЕРЫ_ШРИФТА.length - 1}
            onClick={() => onChange(РАЗМЕРЫ_ШРИФТА[шаг + 1] ?? кегль)}
          >
            A+
          </button>
        </div>
      </div>

      <p className="muted pref-sample" style={{ fontSize: `${кегль}px` }}>{t("Образец: Проверка распознавания речи. Сегодня вторник.")}
      </p>
    </section>
  );
}

function ModelDock({ model, onChange }: { model: ModelStatus; onChange: (next: ModelStatus) => void }) {
  return (
    <aside className="model-dock" aria-label="Состояние модели">
      <div className="model-dock-kicker">// MODEL.NODE // STATIC LINK</div>
      <ModelBar model={model} onChange={onChange} />
      <div className="model-dock-voice">
        <div className="model-dock-kicker">// AUDIO.SUBSYSTEM</div>
        <VoiceToggle />
      </div>
    </aside>
  );
}

/**
 * Цвет смысла раздела.
 *
 * Золото — действие, которое запускает работу: модель, загрузка,
 * знакомство. Фиолетовый — сам секретарь, то, что думает и
 * действует само. Синий — передача данных: чат и профиль.
 *
 * Цвет здесь код, а не украшение. Подпись читается медленно и
 * требует фокуса внимания, а точка под вкладкой видна боковым
 * зрением, и человек узнаёт раздел раньше, чем прочитает его.
 */
const ТОН: Record<Tab, "gold" | "blue"> = {
  agent: "gold",
  chat: "blue",
  download: "gold",
  capabilities: "blue",
  settings: "blue",
  skills: "gold",
  dashboard: "blue",
};

/** Состояние голоса, как его отдаёт `voice_status`. */
interface VoiceState {
  on: boolean;
  listening: boolean;
  weightMb: number | null;
}

/**
 * Переключатель голоса.
 *
 * Голос включается только по нажатию человека, и это не удобство, а
 * необходимость: поднятое распознавание занимает место, которое на
 * 8 ГиБ нужно агенту. Выключенный голос не занимает ничего — поэтому
 * он и выключен по умолчанию, а человек включает его на время разговора.
 */
/** Устройство звука в том виде, как его отдаёт Rust. */
type AudioDevice = {
  id: string;
  имя: string;
  по_умолчанию: boolean;
};

/** Что человек выбрал для звука. */
type AudioChoice = {
  микрофон: string | null;
  вывод: string | null;
  горячая_клавиша: string | null;
};

type AudioDevices = {
  микрофоны: AudioDevice[];
  выводы: AudioDevice[];
  выбор: AudioChoice | null;
};

/** Тема оформления: режим глитча. */
type AudioTheme = {
  глитч: string | null;
};

/** Выбор устройств звука.
 *
 * Отдельный список на каждое направление, потому что у человека их
 * два независимо: микрофон один, а наушники и колонки — разные
 * устройства, и выбирать их надо по отдельности.
 *
 * Устройство по умолчанию идёт первым и подписан: иначе человек
 * ищет его глазами в списке, который Windows отдаёт в случайном
 * порядке.
 */
function AudioSettings({
  глитч,
  установитьГлитч,
}: {
  глитч: string | null;
  установитьГлитч: (режим: string) => void;
}) {
  const [data, setData] = useState<AudioDevices | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const перечитать = () => {
    invoke<AudioDevices>("voice_devices").then(setData).catch(() => undefined);
  };

  useEffect(() => {
    перечитать();
  }, []);

  async function сохранить(микрофон: string, вывод: string) {
    setBusy(true);
    setNote(null);
    try {
      await invoke<AudioChoice>("voice_audio_set", {
        микрофон: микрофон || null,
        вывод: вывод || null,
        горячая_клавиша: data?.выбор?.горячая_клавиша ?? null,
      });
      перечитать();
    } catch (e) {
      setNote(String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Один список выбора. */
  function список(
    устройства: AudioDevice[],
    выбранное: string,
    метка: string,
    подпись: string,
    onChange: (id: string) => void,
  ) {
    return (
      <div className="audio-row">
        <label className="audio-label" htmlFor={`audio-${метка}`}>{подпись}</label>
        <select
          id={`audio-${метка}`}
          className="audio-select"
          value={выбранное}
          disabled={busy || устройства.length === 0}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">{t("По умолчанию Windows")}</option>
          {устройства.map((устройство) => (
            <option key={устройство.id} value={устройство.id}>
              {устройство.имя || t("Без имени")}
              {устройство.по_умолчанию ? ` — ${t("по умолчанию")}` : ""}
            </option>
          ))}
        </select>
      </div>
    );
  }

  /** Переключатель режима глитча: три кнопки, активная подсвечена. */
  function переключательГлитча(текущий: string | null) {
    const режимы: Array<[string, string]> = [
      ["none", t("Выкл")],
      ["subtle", t("Тонко")],
      ["aggressive", t("Агрессивно")],
    ];
    return (
      <div className="glitch-toggle">
        <label className="audio-label">{t("Глитч-эффекты")}</label>
        <div className="glitch-buttons">
          {режимы.map(([значение, метка]) => (
            <button
              key={значение}
              type="button"
              className={`glitch-btn${текущий === значение ? " glitch-btn-active" : ""}`}
              disabled={busy}
              onClick={() => установитьГлитч(значение)}
            >
              {метка}
            </button>
          ))}
        </div>
      </div>
    );
  }

  if (!data) return <section className="panel"><h2>{t("Звук")}</h2><p className="muted">{t("устройства звука: …")}</p></section>;

  const микрофон = data.выбор?.микрофон ?? "";
  const вывод = data.выбор?.вывод ?? "";

  return (
    <section className="panel">
      <h2>{t("Звук")}</h2>
      <p className="muted">{t("Запись и голос идут на те устройства, которые ты выберешь. Системное устройство по умолчанию не меняется — это остаётся в панели Windows.")}</p>

      {список(data.микрофоны, микрофон, "mic", t("Микрофон"), (id) => сохранить(id, вывод))}
      {список(data.выводы, вывод, "out", t("Вывод"), (id) => сохранить(микрофон, id))}

      {переключательГлитча(глитч ?? "none")}

      {data.микрофоны.length === 0 && (
        <p className="error">{t("В системе нет ни одного микрофона. Запись работать не будет.")}</p>
      )}
      {data.выводы.length === 0 && (
        <p className="error">{t("В системе нет ни одного устройства вывода. Голос не прозвучит.")}</p>
      )}
      {note && <p className="error">{note}</p>}
    </section>
  );
}

function VoiceToggle() {
  const [state, setState] = useState<VoiceState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<VoiceState>("voice_status").then(setState).catch(() => undefined);
  }, []);

  const переключить = async () => {
    setBusy(true);
    setError(null);
    try {
      const следующее = state?.on
        ? await invoke<VoiceState>("voice_disable")
        : await invoke<VoiceState>("voice_enable");
      setState(следующее);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!state) return <span className="voice">{t("голос: …")}</span>;

  return (
    <span className="voice">
      <button
        type="button"
        className={state.on ? "chip on" : "chip"}
        disabled={busy}
        onClick={переключить}
        title={state.on
          ? t("выключить голос и освободить место")
          : t("включить голос: поднимется распознавание")}
      >
        {busy ? "…" : state.on ? t("голос включён") : t("включить голос")}
      </button>
      {state.on && state.weightMb !== null && (
        <span className="voice-mb">{state.weightMb} МБ</span>
      )}
      {error && <span className="voice-error" title={error}>{t("не вышло")}</span>}
    </span>
  );
}

/** Снимок памяти, как его отдаёт команда `память_сейчас`. */
interface MemoryView {
  ramUsedMb: number;
  ramTotalMb: number;
  vramUsedMb: number | null;
  vramTotalMb: number | null;
  vramНеизвестна: boolean;
}

/** Гигабайты для подписи, одна значащая цифра после запятой. */
function гиг(миб: number): string {
  return (миб / 1024).toFixed(1);
}

/**
 * Полоса памяти в подвале.
 *
 * Показывает оперативную всегда, видеопамять — только если счётчик
 * ответил. На машине без NVIDIA видеопамяти в этом смысле нет, и
 * пустая полоса вводила бы в заблуждение: выглядело бы как «занято
 * 0 из 0», то есть как поломка.
 *
 * Опрос раз в две секунды. Чаще — заметно на глаз, когда полоса
 * дёргается; реже — человек не видит, что место кончилось, вплоть до
 * отказа модели.
 */
function MemoryBar() {
  const [mem, setMem] = useState<MemoryView | null>(null);

  useEffect(() => {
    let жив = true;
    const опрос = async () => {
      try {
        const снимок = await invoke<MemoryView>("память_сейчас");
        if (жив) setMem(снимок);
      } catch {
        // Снимок не пришёл: полоса просто остаётся как была.
        // Ошибку здесь не показываем — она повторилась бы каждые
        // две секунды и забила собой подвал.
      }
    };
    опрос();
    const таймер = setInterval(опрос, 2000);
    return () => {
      жив = false;
      clearInterval(таймер);
    };
  }, []);

  if (!mem || mem.ramTotalMb === 0) {
    return <span className="mem">{t("память: …")}</span>;
  }

  const ram = Math.min(100, Math.round((mem.ramUsedMb * 100) / mem.ramTotalMb));
  const vram = mem.vramНеизвестна || !mem.vramTotalMb
    ? null
    : Math.min(100, Math.round(((mem.vramUsedMb ?? 0) * 100) / mem.vramTotalMb));

  return (
    <span className="mem">
      <span className="mem-item" title={`оперативная память: ${гиг(mem.ramUsedMb)} из ${гиг(mem.ramTotalMb)} ГиБ`}>
        <span className="mem-label">{t("ОЗУ")}</span>
        <span className="bar"><span className="fill" style={{ width: `${ram}%` }} /></span>
        <span className="mem-num">{ram}%</span>
      </span>
      {vram !== null && (
        <span className="mem-item" title={`видеопамять: ${гиг(mem.vramUsedMb ?? 0)} из ${гиг(mem.vramTotalMb ?? 0)} ГиБ`}>
          <span className="mem-label">VRAM</span>
          <span className="bar"><span className="fill" style={{ width: `${vram}%` }} /></span>
          <span className="mem-num">{vram}%</span>
        </span>
      )}
    </span>
  );
}

/**
 * Переключатель языка.
 *
 * Кнопка, а не список: двух языка ровно два, и список ради двух
 * пунктов превращает переключатель в лишний шаг.
 *
 * Подпись на кнопке всегда «на том языке, который получишь»:
 * RU надписью «EN» ждёт нажатия, RU надписью «RU» не ждёт ничего и
 * читается как «уже русский». Так человек не гадает, что нажатие
 * сделает.
 */
function ЯзыкКнопка({ onChange }: { onChange?: (language: "ru" | "en") => void }) {
  const текущийЯзык = язык();
  return (
    <button
      type="button"
      className="lang"
      title={текущийЯзык === "ru" ? "Switch to English" : "Переключить на русский"}
      aria-label={текущийЯзык === "ru" ? "Switch to English" : "Переключить на русский"}
      onClick={() => {
        const новый = текущийЯзык === "ru" ? "en" : "ru";
        применитьЯзык(новый);
        onChange?.(новый);
      }}
    >
      {текущийЯзык === "ru" ? "EN" : "RU"}
    </button>
  );
}

interface AuthSession {
  user: { id: string; login: string; displayName: string };
  token: string;
  rememberMe: boolean;
}

interface AuthProfileSummary {
  login: string;
  displayName: string;
}

interface AuthProfileDeleteResult {
  login: string;
  privateDataRemoved: boolean;
}

function AuthGate({ onAuthenticated }: { onAuthenticated: (session: AuthSession) => void }) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [remember, setRemember] = useState(false);
  const [rememberSupported, setRememberSupported] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [profiles, setProfiles] = useState<AuthProfileSummary[]>([]);
  const [profilesError, setProfilesError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<AuthProfileSummary | null>(null);
  const [deletePassword, setDeletePassword] = useState("");
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deletingProfile, setDeletingProfile] = useState(false);

  useEffect(() => {
    invoke<boolean>("auth_remember_supported")
      .then(setRememberSupported)
      .catch(() => setRememberSupported(false));
    invoke<AuthProfileSummary[]>("auth_profiles_list")
      .then(setProfiles)
      .catch((reason) => setProfilesError(String(reason)));
  }, []);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const session = await invoke<AuthSession>(mode === "login" ? "auth_login" : "auth_register", mode === "login"
        ? { login, password, rememberMe: remember }
        : { login, password, displayName });
      onAuthenticated(session);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function deleteProfile() {
    if (!deleteTarget || !deletePassword) return;
    setDeletingProfile(true);
    setDeleteError(null);
    try {
      const result = await invoke<AuthProfileDeleteResult>("auth_profile_delete", {
        login: deleteTarget.login,
        password: deletePassword,
      });
      setProfiles((current) => current.filter((profile) => profile.login !== result.login));
      setDeleteTarget(null);
      setDeletePassword("");
      setError(null);
      setLogin((current) => current === result.login ? "" : current);
      if (!result.privateDataRemoved) {
        setError(t("Аккаунт удалён, но его приватные файлы не удалось полностью очистить. Обратитесь к администратору устройства."));
      }
    } catch (reason) {
      setDeleteError(String(reason));
      setDeletePassword("");
    } finally {
      setDeletingProfile(false);
    }
  }

  return (
    <main className="auth-screen">
      <section className="auth-card">
        <div className="brand-mark cp-glitch" data-glitch-text="ARKHA">ARKHA</div>
        <p className="app-subtitle">{t("локальный секретарь")}</p>
        <h1>{mode === "login" ? t("Вход") : t("Создать пользователя")}</h1>
        {mode === "register" && <input placeholder={t("Имя")} value={displayName} onChange={(event) => setDisplayName(event.target.value)} />}
        <input placeholder="login" value={login} onChange={(event) => setLogin(event.target.value)} autoComplete="username" />
        <input placeholder={t("Пароль")} type="password" value={password} onChange={(event) => setPassword(event.target.value)} autoComplete={mode === "login" ? "current-password" : "new-password"} />
        {mode === "login" && rememberSupported && <label className="auth-check"><input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} /> {t("Запомнить меня")}</label>}
        {error && <p className="error">{error}</p>}
        <button type="button" disabled={busy} onClick={() => void submit()}>{mode === "login" ? t("Войти") : t("Создать пользователя")}</button>
        <button type="button" className="link" onClick={() => { setMode(mode === "login" ? "register" : "login"); setError(null); }}>
          {mode === "login" ? t("Создать пользователя") : t("Уже есть пользователь")}
        </button>
        <button type="button" className="link" onClick={() => invoke<AuthSession>("auth_guest").then(onAuthenticated).catch((reason) => setError(String(reason)))}>
          {t("Войти как гость")}
        </button>
        {profilesError && <p className="muted">{profilesError}</p>}
        {mode === "login" && profiles.length > 0 && <section className="auth-profiles" aria-labelledby="auth-profiles-title">
          <h2 id="auth-profiles-title">{t("Профили на этом устройстве")}</h2>
          <ul>
            {profiles.map((profile) => (
              <li key={profile.login} className="auth-profile-row">
                <button type="button" className="auth-profile-select" onClick={() => setLogin(profile.login)}>
                  <strong>{profile.displayName || profile.login}</strong>
                  <span>{profile.login}</span>
                </button>
                <button type="button" className="auth-profile-delete" onClick={() => {
                  setDeleteTarget(profile);
                  setDeletePassword("");
                  setDeleteError(null);
                }}>
                  {t("Удалить профиль")}
                </button>
              </li>
            ))}
          </ul>
        </section>}
      </section>
      {deleteTarget && <div className="profile-delete-backdrop">
        <section className="profile-delete-dialog" role="dialog" aria-modal="true" aria-labelledby="profile-delete-title">
          <h2 id="profile-delete-title">{t("Удалить профиль")}: {deleteTarget.displayName || deleteTarget.login}</h2>
          <p>{t("Будут удалены аккаунт и его настройки, память, вложения и навыки. Общие модели и runtime останутся.")}</p>
          <label htmlFor="profile-delete-password">{t("Для подтверждения введите пароль этого профиля")}</label>
          <input
            id="profile-delete-password"
            type="password"
            autoComplete="current-password"
            value={deletePassword}
            onChange={(event) => setDeletePassword(event.target.value)}
            onKeyDown={(event) => { if (event.key === "Enter") void deleteProfile(); }}
            autoFocus
          />
          {deleteError && <p className="error" role="alert">{deleteError}</p>}
          <div className="profile-delete-actions">
            <button type="button" className="link" disabled={deletingProfile} onClick={() => {
              setDeleteTarget(null);
              setDeletePassword("");
              setDeleteError(null);
            }}>{t("Отмена")}</button>
            <button type="button" className="danger" disabled={deletingProfile || !deletePassword} onClick={() => void deleteProfile()}>
              {deletingProfile ? t("Удаляем…") : t("Удалить аккаунт навсегда")}
            </button>
          </div>
        </section>
      </div>}
    </main>
  );
}

function App() {
  const [session, setSession] = useState<AuthSession | null>(null);
  const [checking, setChecking] = useState(true);

  useEffect(() => {
    // Remove the old plaintext token left by previous ArkHa versions.
    localStorage.removeItem("arkha-session-token");
    invoke<AuthSession>("auth_session")
      .then(setSession)
      .catch(() => undefined)
      .finally(() => setChecking(false));
  }, []);

  if (checking) return <main className="auth-screen"><p>{t("Проверяем сессию…")}</p></main>;
  if (!session) return <AuthGate onAuthenticated={setSession} />;
  return <LegacyMigrationGate guest={session.user.login === "guest"}>
    <Workspace session={session} onLogout={async () => {
    try {
      await invoke("auth_logout", { token: session.token });
      localStorage.removeItem("arkha-session-token");
      setSession(null);
    } catch (reason) {
      console.error("logout failed", reason);
    }
    }} />
  </LegacyMigrationGate>;
}

function Workspace({ session, onLogout }: { session: AuthSession; onLogout: () => void }) {
  const [profile, setProfile] = useState<ProfileView | null>(null);
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [agentMode, setAgentMode] = useState<"plan" | "think" | "build">("build");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [fontSize, setFontSize] = useState(14);

  // Отдельное окно RAG (дообучение корпусом)
  const isRagWindow = getCurrentWindow().label === "rag";
  if (isRagWindow) {
    return <RagWindow />;
  }

  // Отдельное окно выбора агента
  const isAgentSelectWindow = getCurrentWindow().label === "agent_select";
  if (isAgentSelectWindow) {
    return <AgentSelect />;
  }
  const isModelsWindow = getCurrentWindow().label === "models";
  if (isModelsWindow) {
    return <main className="app-shell models-window"><ModelLibrary /></main>;
  }
  // Слушатель события rag:ask — текст из RAG-окна для чата
  useEffect(() => {
    const unlisten = listen("rag:ask", (event) => {
      const text = event.payload as string;
      // Добавляем текст в сообщение чата (открываем вкладку чат)
      setTab("chat");
      // Отправляем сообщение
      invoke("chat", { messages: [{ role: "user", content: `Объясни: ${text}` }] }).catch(() => {});
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Пустое состояние, которое меняется при смене языка: `t()` читает
  // модульную переменную, и React об этом не знает. Подписка шлёт
  // событие, подписчик меняет состояние, дерево перерисовывается.
  const [, перерисовать] = useState(0);
  const [глитчРежим, setГлитчРежим] = useState<string | null>(null);

  useEffect(() => {
    invoke<AppSettings>("settings_get")
      .then((loaded) => {
        setSettings(loaded);
        setFontSize(loaded.font_size);
        setAgentMode(loaded.agent_mode);
        применитьЯзык(loaded.language);
        document.documentElement.style.setProperty("--кегль", `${loaded.font_size}px`);
      })
      .catch(() => {
        setError(t("Не удалось загрузить настройки профиля"));
      });

    return следить(() => перерисовать((n) => n + 1));
  }, []);

  async function saveSettings(next: AppSettings) {
    try {
      const saved = await invoke<AppSettings>("settings_save", { settings: next });
      setSettings(saved);
      setFontSize(saved.font_size);
      setAgentMode(saved.agent_mode);
      применитьЯзык(saved.language);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    invoke<AudioTheme>("voice_theme_settings").then((т) => setГлитчРежим(т.глитч ?? null)).catch(() => undefined);
    invoke<ProfileView>("get_profile").then(setProfile).catch((e) => setError(String(e)));
    invoke<ModelStatus>("model_status").then(setModel).catch(() => undefined);

    // Модель может упасть сама: не хватило видеопамяти, сбойнул
    // драйвер. Без опроса интерфейс вечно показывал бы «запущена» и
    // кнопку «остановить», которая не останавливает ничего, потому
    // что останавливать уже нечего. Пять секунд — достаточно часто,
    // чтобы падение заметил человек, и достаточно редко, чтобы не
    // дёргать окно.
    const опрос = setInterval(() => {
      invoke<ModelStatus>("model_status")
        .then(setModel)
        .catch(() => undefined);
    }, 5000);
    return () => clearInterval(опрос);
  }, []);

  async function установитьГлитч(режим: string) {
    try {
      const следующий = await invoke<AudioTheme>("voice_theme_set", { глитч: режим });
      setГлитчРежим(следующий.глитч ?? null);
    } catch (e) {
      setError(String(e));
    }
  }

  const главныйКласс = `app glitch-on${глитчРежим && глитчРежим !== "none" ? ` glitch-${глитчРежим}` : ""}`;

  if (!model) {
    return (
      <main style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100vh", width: "100%", background: "var(--cp-bg, #000)" }}>
        <p style={{ color: "var(--cp-neon-green, #00ff41)", fontSize: "1.5rem", fontFamily: "var(--cp-font-mono, monospace)" }}>{t("Поднимаем интерфейс…")}</p>
      </main>
    );
  }

  return (
    <main className={главныйКласс.replace(" glitch-on", "")}>
      <div className="cp-scanlines" />
      <div className="cp-corner cp-corner--tl" />
      <div className="cp-corner cp-corner--tr" />
      <div className="cp-corner cp-corner--bl" />
      <div className="cp-corner cp-corner--br" />

      {/* Левая панель (sidebar) */}
      <aside className={`app-sidebar ${sidebarCollapsed ? "collapsed" : ""}`}>
        <div className="sidebar-head">
          {!sidebarCollapsed && (
            <div className="sidebar-brand">
              <span className="brand-mark cp-glitch" data-glitch-text="ARKHA">ARKHA</span>
              <span className="app-subtitle">{t("локальный секретарь")}</span>
            </div>
          )}
          <button
            type="button"
            className="sidebar-toggle"
            onClick={() => setSidebarCollapsed(!sidebarCollapsed)}
            aria-label={sidebarCollapsed ? t("развернуть панель") : t("свернуть панель")}
            aria-expanded={!sidebarCollapsed}
          >
            {sidebarCollapsed ? "»" : "«"}
          </button>
        </div>
        <nav className="sidebar-nav">
          {(["agent", "chat", "download", "settings", "skills", "capabilities", "dashboard"] as Tab[]).map((вкладка) => (
            <Fragment key={вкладка}>
            <button type="button"
              data-tone={ТОН[вкладка]}
              className={tab === вкладка ? "sidebar-item active" : "sidebar-item"}
              onClick={() => setTab(вкладка)}
              title={t({
                agent: "секретарь",
                chat: "чат",
                download: "загрузка",
                capabilities: "возможности",
                settings: "профиль",
                skills: "Навыки",
                dashboard: "наблюдение",
              }[вкладка])}
            >
              <span className="sidebar-icon" aria-hidden="true">{{
                agent: "◈",
                chat: "▤",
                download: "↓",
                capabilities: "✦",
                settings: "◉",
                skills: "⌘",
                dashboard: "▥",
              }[вкладка]}</span>
              {!sidebarCollapsed && <span className="sidebar-label">{t({
                agent: "секретарь",
                chat: "чат",
                download: "загрузка",
                capabilities: "возможности",
                settings: "профиль",
                skills: "Навыки",
                dashboard: "наблюдение",
              }[вкладка])}</span>}
            </button>
            {вкладка === "download" && (
              <button type="button" className="sidebar-item sidebar-subitem" onClick={() => void invoke("models_open_window")}>
                <span className="sidebar-icon" aria-hidden="true">▦</span>
                {!sidebarCollapsed && <span className="sidebar-label">{t("модели")}</span>}
              </button>
            )}
            </Fragment>
          ))}
        </nav>
        {!sidebarCollapsed && (
          <div className="sidebar-agent-select">
            <span className="sidebar-label">{t("Режим агента")}</span>
            <select
              className="agent-mode-select"
              value={agentMode}
              onChange={(e) => setAgentMode(e.target.value as "plan" | "think" | "build")}
              title={t("Режим агента")}
            >
              <option value="plan">{t("plan")}</option>
              <option value="think">{t("think")}</option>
              <option value="build">{t("build")}</option>
            </select>
          </div>
        )}
        <div className="sidebar-footer">
          {!sidebarCollapsed && <div className="sidebar-user">
            <span className="sidebar-label">{session.user.displayName}{session.user.login === "guest" ? ` · ${t("гость")}` : ""}</span>
            <button type="button" className="sidebar-logout" onClick={onLogout}>{t("Выйти")}</button>
          </div>}
          {!sidebarCollapsed && <ЯзыкКнопка onChange={(language) => {
            if (settings) {
              void saveSettings({ ...settings, language, voice: { ...settings.voice, language } });
            }
          }} />}
          {!sidebarCollapsed && <MemoryBar />}
        </div>
      </aside>

      {/* Основной контент */}
      <div className="app-main">
        <header className="app-header">
          <div className="res-strip">
            <span className="res">
              <span className="res-label">{t("профиль")}</span>
              <span className="res-val gold">{profile ? profile.effective : "…"}</span>
            </span>
            <span className="res">
              <span className="res-label">{t("модель")}</span>
              <span className={`res-val ${model.running ? "ok" : "muted"}`}>
                {model.running ? (model.recipe ?? t("запущена")) : t("не запущена")}
              </span>
            </span>
            {model.running && model.tok_s !== null && model.tok_s !== undefined && (
              <span className="res">
                <span className="res-label">{t("скорость")}</span>
                <span className="res-val">{model.tok_s.toFixed(1)} ток/с</span>
              </span>
            )}
            <MemoryBar />
          </div>
        </header>

        <div className="app-workspace">
          <section className="app-body cp-draft-frame">
            {error && <p className="error">Не удалось определить профиль: {error}</p>}
          {/* Агент остаётся смонтированным, иначе его история и контекст
              исчезают при переходе на другую вкладку. */}
          <Agent hidden={tab !== "agent"} />
          {/* Чат не размонтируется: иначе история переписки пропадала
              бы при каждом переходе на другую вкладку. Скрываем его
              через display, состояние остаётся живым. */}
          <Chat model={model} hidden={tab !== "chat"} />
          {tab === "download" && (
            <>
              <ModelLibrary />
              <EnginePanel />
              <RuntimePanel />
              <Downloader model={model} />
            </>
          )}
          {tab === "capabilities" && <Capabilities />}
            {tab === "settings" && profile && (
              <>
                <Settings profile={profile} onChange={setProfile} />
                <LocalUserProfile settings={settings} onSave={(next) => void saveSettings(next)} />
                <Welcome />
                <РазмерШрифта
                  кегль={fontSize}
                  onChange={(value) => {
                    setFontSize(value);
                    if (settings) void saveSettings({ ...settings, font_size: value });
                  }}
                />
                <AudioSettings глитч={глитчРежим} установитьГлитч={установитьГлитч} />
              </>
            )}
            {tab === "skills" && <SkillsPanel />}
            {tab === "dashboard" && <Dashboard />}
          </section>
          <ModelDock model={model} onChange={setModel} />
        </div>

        <footer className="app-footer">
           <span className="hint"><kbd className="cap">1</kbd>–<kbd className="cap">8</kbd> вкладок</span>
          <span className="hint"><kbd className="cap">Enter</kbd> отправить</span>
          <span className="hint"><kbd className="cap">Esc</kbd> остановить</span>
          <span className="hint dim">{t("всё считается на этой машине, облако выключено")}</span>
          <span className="hint"><button type="button" onClick={() => invoke("rag_open_window")} style={{ background: "transparent", border: "none", cursor: "pointer", color: "inherit", fontSize: "inherit" }}>📚 RAG</button></span>
          <span className="hint"><button type="button" onClick={() => invoke("agent_select_open_window")} style={{ background: "transparent", border: "none", cursor: "pointer", color: "inherit", fontSize: "inherit" }}>🤖 Агент</button></span>
        </footer>
      </div>
    </main>
  );
}

export default App;
